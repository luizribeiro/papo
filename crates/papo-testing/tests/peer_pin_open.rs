//! Highest-layer coverage for blocking direct opens with stored peer pins.

mod support {
    pub mod direct;
    pub mod peer_pin;
}

use papo::{Error, Event, PeerKeyCheck, PeerKeyMismatch, Reconnect};
use papo_testing::wire;
use support::{direct::connected, peer_pin::join};

#[tokio::test]
async fn pinned_direct_open_blocks_identity_changes() {
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
    let stored = client.direct("alice").await.unwrap().peer_key().clone();
    assert!(client.direct_pinned("alice", &stored).await.is_ok());

    wire.server(":alice!~alice@cloak.example ACCOUNT replacement-account")
        .await;
    assert!(matches!(
        events.next().await,
        Some(Event::AccountChanged(_))
    ));
    let error = client.direct_pinned("alice", &stored).await.unwrap_err();
    let Error::PeerPinMismatch(mismatch) = error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(mismatch.reason(), PeerKeyMismatch::DifferentAccount);
    assert!(matches!(
        stored.check(mismatch.current_key()),
        PeerKeyCheck::Mismatch(PeerKeyMismatch::DifferentAccount)
    ));

    let accepted = client.direct("alice").await.unwrap();
    assert!(matches!(
        mismatch.current_key().check(accepted.peer_key()),
        PeerKeyCheck::Match
    ));

    let replacement = accepted.peer_key().clone();
    wire.server(":alice!~alice@cloak.example ACCOUNT *").await;
    assert!(matches!(
        events.next().await,
        Some(Event::AccountChanged(_))
    ));
    let Error::PeerPinMismatch(unverified) = client
        .direct_pinned("alice", &replacement)
        .await
        .unwrap_err()
    else {
        panic!("expected a pin mismatch");
    };
    assert_eq!(unverified.reason(), PeerKeyMismatch::CannotVerify);
    assert!(!unverified.current_key().is_verified());
}
