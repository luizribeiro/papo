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
        let state = alice.snapshot();
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
        assert!(alice.snapshot().isupport().is_channel(&name));

        alice.join(&name).await?;
        bob.join(&name).await?;
        let joined = alice
            .wait_for(|event| {
                event.as_joined().filter(|joined| {
                    joined.channel.name().as_str() == name
                        && joined.actor.nick() == bob_nick.as_str()
                })
            })
            .await?;
        assert_eq!(joined.channel.name().as_str(), name);
        assert_eq!(joined.actor.nick(), bob_nick.as_str());
        assert_eq!(joined.authorship, papo::Authorship::Other);

        let text = format!("hello {c} channel");
        bob.privmsg(&name, text.clone()).await?;
        let message = alice
            .wait_for(|event| event.as_message().filter(|message| message.text == text))
            .await?;
        assert_eq!(
            crate::scenarios::support::conversation_name(message.conversation()),
            Some(name.clone())
        );
        assert!(!crate::scenarios::support::conversation_is_private(
            message.conversation()
        ));
        assert_eq!(message.return_address(), name.as_str());
        assert_eq!(crate::source_nick(&message.actor), Some(bob_nick.as_str()));
    }

    for c in UNADVERTISED_CHANTYPES_CANDIDATES {
        if types.contains(c) {
            continue;
        }
        let name = format!("{c}{stem}");
        assert!(!alice.snapshot().isupport().is_channel(&name));

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
        assert!(alice.snapshot().__channel_local(&name).is_none());
    }

    let direct = "a direct message";
    let alice_direct = bob.query(alice_nick.as_str())?;
    bob.privmsg(&alice_direct, direct).await?;
    let private = alice
        .wait_for(|event| event.as_message().filter(|message| message.text == direct))
        .await?;
    assert!(crate::scenarios::support::conversation_is_private(
        private.conversation()
    ));
    assert_eq!(
        crate::scenarios::support::conversation_name(private.conversation()),
        Some(bob_nick.to_string())
    );
    assert_eq!(private.return_address(), bob_nick.as_str());

    assert_eq!(alice.snapshot().channels().count(), types.chars().count());
    alice.expect_none(Event::as_joined).await?;
    Ok(())
}
