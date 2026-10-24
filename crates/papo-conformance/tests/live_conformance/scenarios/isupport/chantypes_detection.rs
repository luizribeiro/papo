use papo::{Error, Event, InvalidIdentifier, InvalidInput, isupport::Advertised};
use papo_conformance_macros::scenario;

use crate::Scenario;

const UNADVERTISED_CHANTYPES_CANDIDATES: [char; 4] = ['#', '&', '!', '+'];

/// treats only advertised CHANTYPES prefixes as channels
#[scenario(covers = ["base.isupport.chantypes"])]
async fn chantypes_detection(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();

    let types = {
        let state = alice.state();
        let Advertised::Value(raw) = state.isupport().get("CHANTYPES") else {
            panic!("the server did not advertise a CHANTYPES value");
        };
        assert_eq!(state.isupport().chantypes(), raw);
        raw.to_owned()
    };

    let channel = s.channel_name();
    let stem = channel.trim_start_matches('#');

    for c in types.chars() {
        let name = format!("{c}{stem}");
        assert!(alice.state().isupport().is_channel(&name));

        alice.join(&name).await?;
        bob.join(&name).await?;
        let joined = alice
            .wait_for(|event| {
                event.as_joined().filter(|joined| {
                    joined.channel.as_ref() == name && joined.nick == bob_nick.as_ref()
                })
            })
            .await?;
        assert_eq!(joined.channel.as_ref(), name);
        assert_eq!(joined.nick, bob_nick.as_ref());
        assert_eq!(joined.authorship, papo::Authorship::Other);

        let text = format!("hello {c} channel");
        bob.privmsg(&name, text.clone()).await?;
        let message = alice
            .wait_for(|event| event.as_message().filter(|message| message.text == text))
            .await?;
        assert_eq!(
            crate::scenarios::support::conversation_name(&message.conversation),
            Some(name.as_str())
        );
        assert!(!crate::scenarios::support::conversation_is_private(
            &message.conversation
        ));
        assert_eq!(message.return_address(), name);
        assert_eq!(crate::source_nick(&message.actor), Some(bob_nick.as_ref()));
    }

    for c in UNADVERTISED_CHANTYPES_CANDIDATES {
        if types.contains(c) {
            continue;
        }
        let name = format!("{c}{stem}");
        assert!(!alice.state().isupport().is_channel(&name));

        // papo rejects an unadvertised prefix locally, so the server never sees this JOIN.
        let error = alice.join(&name).await.unwrap_err();
        match error {
            Error::InvalidInput(InvalidInput::InvalidName {
                name: rejected,
                reason,
                ..
            }) => {
                assert_eq!(rejected, name);
                assert_eq!(reason, InvalidIdentifier::InvalidPrefix(c));
            }
            other => panic!("expected an invalid-prefix rejection for {name:?}, got {other:?}"),
        }
        assert!(alice.state().channel(&name).is_none());
    }

    let direct = "a direct message";
    bob.privmsg(alice_nick.as_ref(), direct).await?;
    let private = alice
        .wait_for(|event| event.as_message().filter(|message| message.text == direct))
        .await?;
    assert!(crate::scenarios::support::conversation_is_private(
        &private.conversation
    ));
    assert_eq!(
        crate::scenarios::support::conversation_name(&private.conversation),
        Some(bob_nick.as_ref())
    );
    assert_eq!(private.return_address(), bob_nick.as_ref());

    assert_eq!(alice.state().channels().count(), types.chars().count());
    alice.expect_none(Event::as_joined).await?;
    Ok(())
}
