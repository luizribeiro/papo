use papo::{Casemapping, Error, Event, Refusal, isupport::Advertised};
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::{
    assert_refusal, conversation_is_private as is_private, conversation_name as name,
};

fn rfc1459_nick(value: &str) -> papo::Nick {
    papo_testing::nick(Casemapping::Rfc1459, value)
}

/// folds []\~ onto {}|^ for nicknames, channels and state lookups under rfc1459
#[scenario(
    requires(isupport = ["CASEMAPPING"]),
    covers = ["base.isupport.casemapping", "base.isupport.casemapping-rfc1459"]
)]
#[allow(clippy::too_many_lines)]
async fn casemapping_rfc1459(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let bob_nick = bob.nick();

    let state = alice.snapshot();
    let support = state.isupport();
    if support.get("CASEMAPPING") != Advertised::Value("rfc1459") {
        return s.skip("server casemapping is not rfc1459");
    }
    assert_eq!(state.casemapping(), Casemapping::Rfc1459);

    let stem = papo_test_server::unique("bot");
    let square_nick = format!("[{stem}]");
    let curly_nick = format!("{{{stem}}}");
    let mut square = s
        .connect_with(s.configure_nick("square", rfc1459_nick(&square_nick).as_str())?)
        .await?;
    let curly = s
        .connect_with(s.configure_nick("curly", rfc1459_nick(&curly_nick).as_str())?)
        .await?;
    assert_eq!(square.snapshot().nick().as_str(), square_nick);
    assert_ne!(curly.snapshot().nick().as_str(), curly_nick);
    assert_eq!(square.snapshot().nick(), curly_nick.as_str());
    let Error::Server(refusal) = curly.set_nick(curly_nick.clone()).await.unwrap_err() else {
        panic!("rfc1459-equivalent nickname did not return a server refusal");
    };
    assert_eq!(refusal.refusal(), Refusal::NicknameInUse);
    assert_eq!(refusal.code(), "433");

    let mixed_channel = format!("#Papo-{stem}");
    let lowercase_channel = format!("#papo-{stem}");
    square.join(&mixed_channel).await?;
    curly.join(&lowercase_channel).await?;
    let joined = square
        .wait_for(|event| {
            event
                .as_joined()
                .filter(|joined| joined.actor.nick().as_str() == curly.nick().as_str())
        })
        .await?;
    assert_eq!(joined.channel.name().as_str(), mixed_channel);
    assert_eq!(
        square
            .snapshot()
            .name_rules()
            .expect("server advertised CASEMAPPING")
            .channel(&mixed_channel),
        lowercase_channel.as_str()
    );
    assert!(
        square
            .snapshot()
            .__channel_local(&lowercase_channel)
            .is_some()
    );
    assert!(curly.snapshot().__channel_local(&mixed_channel).is_some());

    let base = alice.nick().to_string();
    let alice_nick = format!("{base}[x]");
    alice.set_nick(alice_nick.clone()).await?;
    let changed = alice.wait_for(Event::as_nick_changed).await?;
    assert_eq!(changed.old.as_str(), base);
    assert_eq!(changed.actor.nick().as_str(), alice_nick);
    assert_eq!(changed.new.as_str(), alice_nick);
    assert_eq!(changed.authorship, papo::Authorship::Own);

    // ALICE-<n>{X} folds to the same name as alice-<n>[x], so this claim must collide.
    let collision = bob
        .set_nick(format!("{}{{X}}", base.to_ascii_uppercase()))
        .await
        .unwrap_err();
    assert_refusal(collision, Refusal::NicknameInUse, "433");
    assert_eq!(bob.snapshot().nick(), &bob_nick);
    bob.expect_none(Event::as_nick_changed).await?;

    let chan_base = s.channel_name();
    let chan = format!("{chan_base}[x]^y");
    let folded = format!("{}{{X}}~Y", chan_base.to_ascii_uppercase());
    alice.join(&chan).await?;
    bob.join(&folded).await?;
    let bob_join = bob.wait_for(Event::as_joined).await?;
    assert_eq!(bob_join.channel.name().as_str(), chan);
    assert_eq!(bob_join.authorship, papo::Authorship::Own);
    let observed_join = alice
        .wait_for(|event| {
            event
                .as_joined()
                .filter(|joined| joined.actor.nick().as_str() == bob_nick.as_str())
        })
        .await?;
    assert_eq!(observed_join.channel.name().as_str(), chan);
    assert_eq!(observed_join.authorship, papo::Authorship::Other);

    assert_eq!(
        bob.snapshot()
            .__channel_local(&folded)
            .unwrap()
            .name()
            .as_str(),
        chan
    );
    assert_eq!(
        alice
            .snapshot()
            .__channel_local(&folded)
            .unwrap()
            .name()
            .as_str(),
        chan
    );
    let alice_folded_nick = format!("{}{{X}}", base.to_ascii_uppercase());
    let alice_state = alice.snapshot();
    let own_member = alice_state
        .__channel_local(&chan)
        .unwrap()
        .member(alice_folded_nick.as_str())
        .unwrap()
        .unwrap();
    assert_eq!(own_member.nick().as_str(), alice_nick);
    assert_eq!(
        alice
            .snapshot()
            .name_rules()
            .expect("server advertised CASEMAPPING")
            .channel(&chan),
        folded.as_str()
    );
    let text = "folded channel target";
    alice.privmsg(&folded, text).await?;
    let message = bob
        .wait_for(|event| event.as_message().filter(|message| message.text == text))
        .await?;
    assert_eq!(name(message.conversation()), Some(chan.clone()));
    assert_eq!(
        crate::source_nick(&message.actor),
        Some(alice_nick.as_str())
    );
    assert!(!is_private(message.conversation()));

    bob.expect_none(Event::as_message).await?;
    Ok(())
}
