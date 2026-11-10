use papo::{Casemapping, Error, Event, Refusal, isupport::Advertised};
use papo_conformance_macros::scenario;

use crate::Scenario;

fn ascii_nick(value: &str) -> papo::Nick {
    papo_testing::nick(Casemapping::Ascii, value)
}

fn name_rules(state: &papo::State) -> papo::NameRules<'_> {
    state.name_rules().expect("server advertised CASEMAPPING")
}

fn assert_same_channel(state: &papo::State, left: &str, right: &str) {
    assert_eq!(name_rules(state).channel(left), right);
}

/// folds only ASCII letters for nicknames, channels and state lookups under ascii
#[scenario(
    requires(isupport = ["CASEMAPPING"]),
    covers = ["base.isupport.casemapping", "base.isupport.casemapping-ascii"]
)]
// One connection pair must retain both casemapping observations in sequence.
#[allow(clippy::too_many_lines)]
async fn casemapping_ascii(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;

    if alice.snapshot().isupport().get("CASEMAPPING") != Advertised::Value("ascii") {
        return s.skip("server casemapping is not ascii");
    }
    assert_eq!(alice.snapshot().casemapping(), Casemapping::Ascii);

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
            s.connect_with(s.configure_nick("square", ascii_nick(&square_nick).as_str())?)
                .await?,
            s.connect_with(s.configure_nick("curly", ascii_nick(&curly_nick).as_str())?)
                .await?,
        )
    };
    assert_eq!(square.snapshot().nick().as_str(), square_nick);
    assert_eq!(curly.snapshot().nick().as_str(), curly_nick);
    assert_ne!(square.snapshot().nick(), curly_nick.as_str());

    let mixed_channel = format!("#Papo-{stem}");
    let lowercase_channel = format!("#papo-{stem}");
    square.join(&mixed_channel).await?;
    curly.join(&lowercase_channel).await?;
    let joined = square
        .wait_for(|event| {
            event
                .as_joined()
                .filter(|joined| joined.actor.nick().as_str() == curly_nick)
        })
        .await?;
    assert_eq!(joined.channel.name().as_str(), mixed_channel);
    assert_same_channel(&square.snapshot(), &mixed_channel, &lowercase_channel);
    assert!(
        square
            .snapshot()
            .__channel_local(&lowercase_channel)
            .is_some()
    );
    assert!(curly.snapshot().__channel_local(&mixed_channel).is_some());

    let base = alice.nick();
    let bob_original = bob.nick();

    let alice_square_nick = format!("{base}[x]");
    alice.set_nick(alice_square_nick.clone()).await?;
    let alice_changed = alice.wait_for(Event::as_nick_changed).await?;
    assert_eq!(alice_changed.actor.nick(), &base);
    assert_eq!(alice_changed.new.as_str(), alice_square_nick);
    assert_eq!(alice_changed.authorship, papo::Authorship::Own);

    let bob_curly_nick = format!("{}{{X}}", base.as_str().to_ascii_uppercase());
    bob.set_nick(bob_curly_nick.clone()).await?;
    let bob_changed = bob.wait_for(Event::as_nick_changed).await?;
    assert_eq!(bob_changed.actor.nick(), &bob_original);
    assert_eq!(bob_changed.new.as_str(), bob_curly_nick);
    assert_eq!(bob_changed.authorship, papo::Authorship::Own);
    assert_eq!(bob.snapshot().nick(), bob_curly_nick.as_str());

    let bob_square_attempt = format!("{}[X]", base.as_str().to_ascii_uppercase());
    let Error::Server(refusal) = bob.set_nick(bob_square_attempt).await.unwrap_err() else {
        panic!("ascii-folded nickname collision did not return a server refusal");
    };
    assert_eq!(refusal.refusal(), Refusal::NicknameInUse);
    assert_eq!(refusal.code(), "433");
    assert_eq!(bob.snapshot().nick(), bob_curly_nick.as_str());
    bob.expect_none(Event::as_nick_changed).await?;

    let channel_base = s.channel_name();
    let chan = format!("{channel_base}[x]^y");
    let folded = format!("{}{{X}}~Y", channel_base.as_str().to_ascii_uppercase());

    alice.join(&chan).await?;
    bob.join(&folded).await?;

    let alice_joined = alice.wait_for(Event::as_joined).await?;
    assert_eq!(alice_joined.channel.name().as_str(), chan);
    assert_eq!(alice_joined.actor.nick().as_str(), alice_square_nick);
    assert_eq!(alice_joined.authorship, papo::Authorship::Own);

    let bob_joined = bob.wait_for(Event::as_joined).await?;
    assert_eq!(bob_joined.channel.name().as_str(), folded);
    assert_eq!(bob_joined.actor.nick().as_str(), bob_curly_nick);
    assert_eq!(bob_joined.authorship, papo::Authorship::Own);

    alice.expect_none(Event::as_joined).await?;

    assert!(alice.snapshot().__channel_local(&folded).is_none());
    let alice_state = alice.snapshot();
    let found = alice_state
        .__channel_local(chan.as_str().to_ascii_uppercase())
        .expect("ascii folding finds the channel by its letter-uppercased name");
    assert_eq!(found.name().as_str(), chan);
    assert!(bob.snapshot().__channel_local(&chan).is_none());

    let channel_name = name_rules(&alice_state).channel(&chan);
    assert_ne!(channel_name, folded.as_str());
    assert_eq!(channel_name, chan.as_str().to_ascii_uppercase().as_str());

    assert!(
        alice
            .snapshot()
            .__channel_local(&chan)
            .unwrap()
            .member(alice_square_nick.as_str().to_ascii_uppercase())
            .is_some()
    );
    Ok(())
}
