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
    let alice_pin = client.query("alice").unwrap().key();
    let bob_pin = client.query("bob").unwrap().key();
    assert!(alice_pin.is_verified());
    assert!(!bob_pin.is_verified());
    assert_eq!(bob_pin.as_verified_str(), None);

    wire.close().await;
    while !matches!(
        events.next().await.map(|(event, _)| event),
        Some(Event::Disconnected(_))
    ) {}
    wire.register(&caps).await;
    connected(&mut events).await;
    for line in [
        ":papo!p@localhost JOIN #review papo-account :Papo",
        ":alice!~alice@new-cloak.example JOIN #review alice-account :Alice",
        ":bob!~bob@new-users.example JOIN #review * :Bob",
    ] {
        join(&mut events, &mut wire, line).await;
    }

    assert_match(&alice_pin, &client.query("alice").unwrap().key());
    assert_mismatch(
        &bob_pin,
        &client.query("bob").unwrap().key(),
        papo::PeerKeyMismatch::CannotVerify,
    );
}

#[tokio::test]
async fn session_keys_change_on_rename_and_expire_on_reuse_and_logout() {
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

    let alice_pin = client.query("alice").unwrap().key();
    let bob = client.query("bob").unwrap();
    let bob_thread = bob.nick().key().to_owned();
    let bob_pin = bob.key();
    wire.server(":bob!~bob@users.example NICK robert").await;
    let Event::NickChanged(changed) = next_active_event(&mut events).await else {
        panic!("expected nickname change");
    };
    assert_eq!(changed.actor.nick(), "robert");
    assert_eq!(changed.old, "bob");
    assert_eq!(changed.new, "robert");
    let renamed = client.query("robert").unwrap();
    assert_ne!(renamed.nick().key(), bob_thread);
    assert_mismatch(
        &bob_pin,
        &renamed.key(),
        papo::PeerKeyMismatch::CannotVerify,
    );

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
        &client.query("robert").unwrap().key(),
        papo::PeerKeyMismatch::CannotVerify,
    );

    wire.server(":alice!~alice@cloak.example ACCOUNT *").await;
    assert!(matches!(
        next_active_event(&mut events).await,
        Event::AccountChanged(_)
    ));
    let logged_out = client.query("alice").unwrap();
    assert!(!logged_out.key().is_verified());
    assert_mismatch(
        &alice_pin,
        &logged_out.key(),
        papo::PeerKeyMismatch::CannotVerify,
    );
}

#[tokio::test]
async fn departed_handle_key_does_not_adopt_the_next_holder() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("ircs://verified.example", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-notify"]).await;
    connected(&mut events).await;
    join(&mut events, &mut wire, ":papo!p@localhost JOIN #review").await;
    join(
        &mut events,
        &mut wire,
        ":alice!~alice@cloak.example JOIN #review",
    )
    .await;
    let alice = client.query("alice").unwrap();
    let stored = alice.key();
    assert!(!stored.is_verified());

    wire.server(":alice!~alice@cloak.example QUIT :Client exited")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        Event::Quit(_)
    ));
    join(
        &mut events,
        &mut wire,
        ":alice!~mallory@visitor.example JOIN #review",
    )
    .await;
    wire.server(":alice!~mallory@visitor.example ACCOUNT mallory-account")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        Event::AccountChanged(_)
    ));

    assert!(client.query("alice").unwrap().key().is_verified());
    assert!(!alice.key().is_verified());
    assert_mismatch(
        &stored,
        &client.query("alice").unwrap().key(),
        papo::PeerKeyMismatch::CannotVerify,
    );
}
