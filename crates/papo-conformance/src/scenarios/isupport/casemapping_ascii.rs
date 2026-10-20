use papo::{Casemapping, Error, Event, Refusal, isupport::Advertised};
use papo_conformance_macros::scenario;

use crate::Scenario;

fn ascii_nick(value: &str) -> papo::Nick {
    Casemapping::Ascii.nick(value)
}

fn assert_same_channel(state: &papo::State, left: &str, right: &str) {
    assert_eq!(state.channel_name(left), right);
}

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

    let stem = papo_test_server::unique("bot");
    let square_nick = format!("[{stem}]");
    let curly_nick = format!("{{{stem}}}");
    let profile = std::env::var("PAPO_TEST_PROFILE").unwrap_or_default();
    let (mut square, curly) = if profile == "soju-ergo" {
        let square = s.connect("square").await?;
        let curly = s.connect("curly").await?;
        square.set_nick(square_nick.clone()).await?;
        curly.set_nick(curly_nick.clone()).await?;
        (square, curly)
    } else {
        (
            s.connect_with(s.configure("square")?.nick(ascii_nick(&square_nick)))
                .await?,
            s.connect_with(s.configure("curly")?.nick(ascii_nick(&curly_nick)))
                .await?,
        )
    };
    assert_eq!(square.state().nick().as_ref(), square_nick);
    assert_eq!(curly.state().nick().as_ref(), curly_nick);
    assert_ne!(square.state().nick(), curly_nick.as_str());

    let mixed_channel = format!("#Papo-{stem}");
    let lowercase_channel = format!("#papo-{stem}");
    square.join(&mixed_channel).await?;
    curly.join(&lowercase_channel).await?;
    let joined = square
        .wait_for(|event| {
            event
                .as_joined()
                .filter(|joined| joined.nick.as_ref() == curly_nick)
        })
        .await?;
    assert_eq!(joined.channel.as_ref(), mixed_channel);
    assert_same_channel(&square.state(), &mixed_channel, &lowercase_channel);
    assert!(square.state().channel(&lowercase_channel).is_some());
    assert!(curly.state().channel(&mixed_channel).is_some());

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
    assert_eq!(alice_joined.authorship, papo::Authorship::Own);

    let bob_joined = bob.wait_for(Event::as_joined).await?;
    assert_eq!(bob_joined.channel.as_ref(), folded);
    assert_eq!(bob_joined.nick.as_ref(), bob_curly_nick);
    assert_eq!(bob_joined.authorship, papo::Authorship::Own);

    alice.expect_none(Event::as_joined).await?;

    assert!(alice.state().channel(&folded).is_none());
    let alice_state = alice.state();
    let found = alice_state
        .channel(chan.to_ascii_uppercase())
        .expect("ascii folding finds the channel by its letter-uppercased name");
    assert_eq!(found.name().as_ref(), chan);
    assert!(bob.state().channel(&chan).is_none());

    let channel_name = alice.state().channel_name(&chan);
    assert_ne!(channel_name, folded.as_str());
    assert_eq!(channel_name, chan.to_ascii_uppercase().as_str());

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
