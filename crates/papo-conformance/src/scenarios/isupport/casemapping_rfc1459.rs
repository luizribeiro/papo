use papo::{Casemapping, Error, Event, Refusal, Source, isupport::Advertised};
use papo_conformance_macros::scenario;

use crate::Scenario;

/// folds []\~ onto {}|^ for nicknames, channels and state lookups under rfc1459
#[scenario(
    requires(isupport = ["CASEMAPPING"]),
    covers = ["base.isupport.casemapping", "base.isupport.casemapping-rfc1459"]
)]
async fn casemapping_rfc1459(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let bob_nick = bob.nick();

    let state = alice.state();
    let support = state.isupport();
    if support.get("CASEMAPPING") != Advertised::Value("rfc1459") {
        return s.skip("server casemapping is not rfc1459");
    }
    assert_eq!(state.casemapping(), Casemapping::Rfc1459);

    let base = alice.nick().to_string();
    let alice_nick = format!("{base}[x]");
    alice.set_nick(alice_nick.clone()).await?;
    let changed = alice.wait_for(Event::as_nick_changed).await?;
    assert_eq!(changed.old.as_ref(), base);
    assert_eq!(changed.new.as_ref(), alice_nick);
    assert!(changed.is_self);

    // ALICE-<n>{X} folds to the same name as alice-<n>[x], so this claim must collide.
    let Error::Server(refusal) = bob
        .set_nick(format!("{}{{X}}", base.to_ascii_uppercase()))
        .await
        .unwrap_err()
    else {
        panic!("folded nickname collision did not return a server refusal");
    };
    assert_eq!(refusal.kind(), Refusal::NicknameInUse);
    assert_eq!(refusal.code(), "433");
    assert_eq!(bob.state().nick(), &bob_nick);
    bob.expect_none(Event::as_nick_changed).await?;

    let chan_base = s.channel_name();
    let chan = format!("{chan_base}[x]^y");
    let folded = format!("{}{{X}}~Y", chan_base.to_ascii_uppercase());
    alice.join(&chan).await?;
    bob.join(&folded).await?;
    let bob_join = bob.wait_for(Event::as_joined).await?;
    assert_eq!(bob_join.channel.as_ref(), chan);
    assert!(bob_join.is_self);
    let observed_join = alice
        .wait_for(|event| event.as_joined().filter(|joined| joined.nick == bob_nick))
        .await?;
    assert_eq!(observed_join.channel.as_ref(), chan);
    assert!(!observed_join.is_self);

    assert_eq!(bob.state().channel(&folded).unwrap().name().as_ref(), chan);
    assert_eq!(
        alice.state().channel(&folded).unwrap().name().as_ref(),
        chan
    );
    let alice_folded_nick = format!("{}{{X}}", base.to_ascii_uppercase());
    let alice_state = alice.state();
    let own_member = alice_state
        .channel(&chan)
        .unwrap()
        .member(&alice_folded_nick)
        .unwrap();
    assert_eq!(own_member.nick().as_ref(), alice_nick);
    assert!(alice.state().casemapping().eq(&chan, &folded));

    let text = "folded channel target";
    alice.privmsg(&folded, text).await?;
    let message = bob
        .wait_for(|event| event.as_message().filter(|message| message.text() == text))
        .await?;
    assert_eq!(message.target(), chan);
    assert_eq!(
        message.sender.as_ref().and_then(Source::nick),
        Some(alice_nick.as_str())
    );
    assert!(!message.is_private());

    bob.expect_none(Event::as_message).await?;
    Ok(())
}
