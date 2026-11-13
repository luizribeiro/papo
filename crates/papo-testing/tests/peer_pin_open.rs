//! Highest-layer coverage for blocking opens with stored peer keys.

mod support {
    pub mod direct;
    pub mod peer_pin;
}

use papo::{Error, Event, PeerKeyCheck, PeerKeyMismatch, Reconnect};
use papo_testing::wire;
use support::{
    direct::{connected, next_active_event},
    peer_pin::join,
};

#[tokio::test]
async fn open_with_stored_key_blocks_identity_changes() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("ircs://verified.example", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-notify", "extended-join"]).await;
    connected(&mut events).await;
    join(
        &mut events,
        &mut wire,
        ":papo!p@localhost JOIN #review papo-account :Papo",
    )
    .await;
    join(
        &mut events,
        &mut wire,
        ":alice!~alice@cloak.example JOIN #review alice-account :Alice",
    )
    .await;
    let stored = client.query("alice").unwrap().key();
    assert!(client.open(&stored).await.is_ok());

    wire.server(":alice!~alice@cloak.example ACCOUNT replacement-account")
        .await;
    assert!(matches!(
        events.next().await.map(|(event, _)| event),
        Some(Event::AccountChanged(_))
    ));
    let error = client.open(&stored).await.unwrap_err();
    let Error::NotSamePerson(mismatch) = error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(mismatch.reason(), PeerKeyMismatch::DifferentAccount);
    assert!(matches!(
        stored.check(mismatch.current_key()),
        PeerKeyCheck::Mismatch(PeerKeyMismatch::DifferentAccount)
    ));

    let accepted = client.query("alice").unwrap();
    assert!(matches!(
        mismatch.current_key().check(&accepted.key()),
        PeerKeyCheck::Match
    ));

    let replacement = accepted.key();
    wire.server(":alice!~alice@cloak.example ACCOUNT *").await;
    assert!(matches!(
        events.next().await.map(|(event, _)| event),
        Some(Event::AccountChanged(_))
    ));
    let Error::NotSamePerson(unverified) = client.open(&replacement).await.unwrap_err() else {
        panic!("expected a pin mismatch");
    };
    assert_eq!(unverified.reason(), PeerKeyMismatch::CannotVerify);
    assert!(!unverified.current_key().is_verified());
}

#[tokio::test]
async fn bouncer_upstream_drop_rejects_continuity_pin_and_live_target() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("ircs://bouncer.example", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    wire.server(":bouncer.papo.test 005 papo BOUNCER_NETID=1 CASEMAPPING=ascii :are supported")
        .await;
    while !matches!(
        next_active_event(&mut events).await,
        Event::CasemappingChanged(_)
    ) {}
    join(
        &mut events,
        &mut wire,
        ":papo!~u@client.example JOIN #review",
    )
    .await;
    join(
        &mut events,
        &mut wire,
        ":alice!alice-old@cloak.example JOIN #review",
    )
    .await;
    let alice = client.query("alice").unwrap();
    let stored = alice.key();
    assert!(!stored.is_verified());

    wire.server(":BouncerServ!BouncerServ@BouncerServ NOTICE papo :disconnected from ergo")
        .await;
    while !matches!(next_active_event(&mut events).await, Event::Notice(_)) {}
    assert!(matches!(
        client.privmsg(&alice, "release approved").await,
        Err(Error::TargetGone)
    ));

    wire.server(":BouncerServ!BouncerServ@BouncerServ NOTICE papo :connected to ergo")
        .await;
    while !matches!(next_active_event(&mut events).await, Event::Notice(_)) {}
    join(
        &mut events,
        &mut wire,
        ":papo!~u@new-client.example JOIN #review",
    )
    .await;
    wire.server(":bouncer.papo.test 353 * = #review :@alice papo")
        .await;
    wire.server(":bouncer.papo.test 366 * #review :End of /NAMES list")
        .await;
    while !matches!(
        next_active_event(&mut events).await,
        Event::ChannelSynced(_)
    ) {}

    let Error::NotSamePerson(mismatch) = client.open(&stored).await.unwrap_err() else {
        panic!("expected the continuity pin to be rejected");
    };
    assert_eq!(mismatch.reason(), PeerKeyMismatch::CannotVerify);
    assert!(matches!(
        stored.check(mismatch.current_key()),
        PeerKeyCheck::Mismatch(PeerKeyMismatch::CannotVerify)
    ));
}
