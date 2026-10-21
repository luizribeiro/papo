use papo::{Error, Event, InvalidIdentifier, InvalidInput, isupport::Advertised};
use papo_conformance_macros::scenario;

use crate::Scenario;

/// joins a channel named at exactly CHANNELLEN and rejects one character more before sending
#[scenario(requires(isupport = ["CHANNELLEN"]), covers = ["base.isupport.channellen"])]
async fn channellen_limit(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();

    let state = alice.state();
    let support = state.isupport();
    let Advertised::Value(raw) = support.get("CHANNELLEN") else {
        panic!("server did not advertise CHANNELLEN with a value");
    };
    let n: usize = raw.parse().expect("CHANNELLEN value is numeric");
    assert_eq!(support.channellen(), Some(n));

    let base = s.channel_name();
    let padding = n.saturating_sub(base.chars().count());
    let at_limit = format!("{base}{}", "z".repeat(padding));
    assert_eq!(at_limit.chars().count(), n);

    alice.join(&at_limit).await?;
    let self_joined = alice.wait_for(Event::as_joined).await?;
    assert_eq!(self_joined.channel.as_ref(), at_limit);
    assert_eq!(self_joined.nick, alice_nick);
    assert_eq!(self_joined.authorship, papo::Authorship::Own);

    bob.join(&at_limit).await?;
    let self_joined = bob.wait_for(Event::as_joined).await?;
    assert_eq!(self_joined.channel.as_ref(), at_limit);
    assert_eq!(self_joined.nick, bob_nick);
    assert_eq!(self_joined.authorship, papo::Authorship::Own);

    let joined = alice.wait_for(Event::as_joined).await?;
    assert_eq!(joined.channel.as_ref(), at_limit);
    assert_eq!(joined.nick, bob_nick);
    assert_eq!(joined.authorship, papo::Authorship::Other);

    for peer in [&alice, &bob] {
        assert_eq!(
            peer.state().channel(&at_limit).unwrap().name().as_ref(),
            at_limit
        );
    }

    let over = format!("{at_limit}w");
    let Error::InvalidInput(InvalidInput::InvalidName { name, reason, .. }) =
        alice.join(&over).await.unwrap_err()
    else {
        panic!("over-limit channel name did not return an invalid-input error");
    };
    assert_eq!(name, over);
    assert_eq!(
        reason,
        InvalidIdentifier::TooLong {
            length: n + 1,
            maximum: n,
        }
    );

    assert!(alice.state().channel(&over).is_none());
    assert_eq!(alice.state().channels().count(), 1);
    alice.expect_none(Event::as_joined).await?;
    bob.expect_none(Event::as_joined).await?;
    Ok(())
}
