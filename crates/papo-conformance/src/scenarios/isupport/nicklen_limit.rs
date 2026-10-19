use papo::{Error, Event, InvalidIdentifier, InvalidInput, isupport::Advertised};
use papo_conformance_macros::scenario;

use crate::Scenario;

/// accepts a nickname of exactly NICKLEN and rejects one character more before sending
#[scenario(requires(isupport = ["NICKLEN"]), covers = ["base.isupport.nicklen"])]
async fn nicklen_limit(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let original = bob.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let joined = alice
        .wait_for(|event| event.as_joined().filter(|joined| joined.nick == original))
        .await?;
    assert_eq!(joined.channel.as_ref(), channel);
    assert_eq!(joined.nick, original);
    assert!(!joined.is_self);

    let state = bob.state();
    let support = state.isupport();
    let Advertised::Value(raw) = support.get("NICKLEN") else {
        panic!("server did not advertise NICKLEN with a value");
    };
    let n: usize = raw.parse().expect("NICKLEN value is numeric");
    assert_eq!(support.nicklen(), Some(n));

    let padding = n.saturating_sub(original.chars().count());
    let at_limit = format!("{original}{}", "q".repeat(padding));
    assert_eq!(at_limit.chars().count(), n);

    bob.set_nick(at_limit.clone()).await?;
    let own = bob.wait_for(Event::as_nick_changed).await?;
    let observed = alice.wait_for(Event::as_nick_changed).await?;
    for changed in [&own, &observed] {
        assert_eq!(changed.old, original);
        assert_eq!(changed.new.as_ref(), at_limit);
    }
    assert!(own.is_self);
    assert!(!observed.is_self);
    assert_eq!(bob.state().nick(), at_limit.as_str());
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&at_limit)
            .is_some()
    );
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&original)
            .is_none()
    );

    let over = format!("{at_limit}r");
    assert_eq!(over.chars().count(), n + 1);
    let Error::InvalidInput(InvalidInput::InvalidMessage(text)) =
        bob.set_nick(over).await.unwrap_err()
    else {
        panic!("over-limit nickname did not return an invalid-input error");
    };
    assert_eq!(
        text,
        InvalidIdentifier::TooLong {
            length: n + 1,
            maximum: n,
        }
        .to_string()
    );

    bob.expect_none(Event::as_nick_changed).await?;
    alice.expect_none(Event::as_nick_changed).await?;
    assert_eq!(bob.state().nick(), at_limit.as_str());
    Ok(())
}
