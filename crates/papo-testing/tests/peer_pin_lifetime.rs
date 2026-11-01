//! Highest-layer coverage for direct peer pin lifetimes.

mod support {
    pub mod direct;
    pub mod peer_pin;
}

use papo::{Event, Reconnect};
use papo_testing::wire;
use support::{
    direct::{connected, next_active_event},
    peer_pin::{assert_match, assert_mismatch, join, reconnecting},
};

#[tokio::test(start_paused = true)]
async fn verified_pins_survive_reconnect_while_session_pins_do_not() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("ircs://verified.example", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(reconnecting()),
    );
    let caps = ["account-notify", "extended-join"];
    wire.register(&caps).await;
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
    join(
        &mut events,
        &mut wire,
        ":bob!~bob@users.example JOIN #review * :Bob",
    )
    .await;
    let alice_pin = client.direct("alice").await.unwrap().peer_key().clone();
    let bob_pin = client.direct("bob").await.unwrap().peer_key().clone();
    assert!(alice_pin.is_verified());
    assert!(!bob_pin.is_verified());
    assert_eq!(bob_pin.as_verified_str(), None);

    wire.close().await;
    while !matches!(events.next().await, Some(Event::Disconnected(_))) {}
    wire.register(&caps).await;
    connected(&mut events).await;
    for line in [
        ":papo!p@localhost JOIN #review papo-account :Papo",
        ":alice!~alice@new-cloak.example JOIN #review alice-account :Alice",
        ":bob!~bob@new-users.example JOIN #review * :Bob",
    ] {
        join(&mut events, &mut wire, line).await;
    }

    assert_match(&alice_pin, client.direct("alice").await.unwrap().peer_key());
    assert_mismatch(
        &bob_pin,
        client.direct("bob").await.unwrap().peer_key(),
        papo::PeerKeyMismatch::CannotVerify,
    );
}

#[tokio::test]
async fn session_pins_follow_renames_and_expire_on_reuse_and_logout() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("ircs://verified.example", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-notify", "extended-join"]).await;
    connected(&mut events).await;
    for line in [
        ":papo!p@localhost JOIN #review papo-account :Papo",
        ":alice!~alice@cloak.example JOIN #review alice-account :Alice",
        ":bob!~bob@users.example JOIN #review * :Bob",
    ] {
        join(&mut events, &mut wire, line).await;
    }

    let alice_pin = client.direct("alice").await.unwrap().peer_key().clone();
    let bob = client.direct("bob").await.unwrap();
    let bob_thread = bob.nick().key().to_owned();
    let bob_pin = bob.peer_key().clone();
    wire.server(":bob!~bob@users.example NICK robert").await;
    let Event::NickChanged(changed) = next_active_event(&mut events).await else {
        panic!("expected nickname change");
    };
    assert_eq!(changed.actor.nick(), "bob");
    assert_eq!(changed.new, "robert");
    assert_match(&bob_pin, changed.peer_key().unwrap());
    let renamed = client.direct("robert").await.unwrap();
    assert_ne!(renamed.nick().key(), bob_thread);
    assert_match(&bob_pin, renamed.peer_key());

    wire.server(":robert!~bob@users.example QUIT :Client exited")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        Event::Quit(_)
    ));
    join(
        &mut events,
        &mut wire,
        ":robert!~mallory@visitor.example JOIN #review * :Mallory",
    )
    .await;
    assert_mismatch(
        &bob_pin,
        client.direct("robert").await.unwrap().peer_key(),
        papo::PeerKeyMismatch::CannotVerify,
    );

    wire.server(":alice!~alice@cloak.example ACCOUNT *").await;
    assert!(matches!(
        next_active_event(&mut events).await,
        Event::AccountChanged(_)
    ));
    let logged_out = client.direct("alice").await.unwrap();
    assert!(!logged_out.peer_key().is_verified());
    assert_mismatch(
        &alice_pin,
        logged_out.peer_key(),
        papo::PeerKeyMismatch::CannotVerify,
    );
}
