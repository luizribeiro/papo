use papo::{Casemapping, Error, Event, Refusal, isupport::Advertised};
use papo_conformance_macros::scenario;

use crate::Scenario;

/// folds only ASCII letters for nicknames, channels and state lookups under ascii
#[scenario(
    requires(isupport = ["CASEMAPPING"]),
    covers = ["base.isupport.casemapping", "base.isupport.casemapping-ascii"]
)]
async fn casemapping_ascii(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;

    if alice.state().isupport().get("CASEMAPPING") != Advertised::Value("ascii") {
        return s.skip("server casemapping is not ascii");
    }
    assert_eq!(alice.state().casemapping(), Casemapping::Ascii);

    let base = alice.nick();
    let bob_original = bob.nick();

    let alice_square_nick = format!("{base}[x]");
    alice.set_nick(alice_square_nick.clone()).await?;
    let alice_changed = alice.wait_for(Event::as_nick_changed).await?;
    assert_eq!(alice_changed.old, base);
    assert_eq!(alice_changed.new.as_ref(), alice_square_nick);
    assert!(alice_changed.is_self);

    let bob_curly_nick = format!("{}{{X}}", base.to_ascii_uppercase());
    bob.set_nick(bob_curly_nick.clone()).await?;
    let bob_changed = bob.wait_for(Event::as_nick_changed).await?;
    assert_eq!(bob_changed.old, bob_original);
    assert_eq!(bob_changed.new.as_ref(), bob_curly_nick);
    assert!(bob_changed.is_self);
    assert_eq!(bob.state().nick(), bob_curly_nick.as_str());

    let bob_square_attempt = format!("{}[X]", base.to_ascii_uppercase());
    let Error::Server(refusal) = bob.set_nick(bob_square_attempt).await.unwrap_err() else {
        panic!("ascii-folded nickname collision did not return a server refusal");
    };
    assert_eq!(refusal.kind(), Refusal::NicknameInUse);
    assert_eq!(refusal.code(), "433");
    assert_eq!(bob.state().nick(), bob_curly_nick.as_str());
    bob.expect_none(Event::as_nick_changed).await?;

    let channel_base = s.channel_name();
    let chan = format!("{channel_base}[x]^y");
    let folded = format!("{}{{X}}~Y", channel_base.to_ascii_uppercase());

    alice.join(&chan).await?;
    bob.join(&folded).await?;

    let alice_joined = alice.wait_for(Event::as_joined).await?;
    assert_eq!(alice_joined.channel.as_ref(), chan);
    assert_eq!(alice_joined.nick.as_ref(), alice_square_nick);
    assert!(alice_joined.is_self);

    let bob_joined = bob.wait_for(Event::as_joined).await?;
    assert_eq!(bob_joined.channel.as_ref(), folded);
    assert_eq!(bob_joined.nick.as_ref(), bob_curly_nick);
    assert!(bob_joined.is_self);

    alice.expect_none(Event::as_joined).await?;

    assert!(alice.state().channel(&folded).is_none());
    let alice_state = alice.state();
    let found = alice_state
        .channel(chan.to_ascii_uppercase())
        .expect("ascii folding finds the channel by its letter-uppercased name");
    assert_eq!(found.name().as_ref(), chan);
    assert!(bob.state().channel(&chan).is_none());

    let casemapping = alice.state().casemapping();
    assert!(!casemapping.eq(&chan, &folded));
    assert!(casemapping.eq(&chan, &chan.to_ascii_uppercase()));

    assert!(
        alice
            .state()
            .channel(&chan)
            .unwrap()
            .member(alice_square_nick.to_ascii_uppercase())
            .is_some()
    );
    Ok(())
}
