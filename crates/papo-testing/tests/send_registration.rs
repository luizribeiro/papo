//! Offline wire regressions for sends queued during registration.

use std::time::Duration;

use papo::{Line, Reconnect};
use papo_testing::wire;

async fn connected(events: &mut papo::Events) {
    while !matches!(events.next().await, Some(papo::Event::Connected(_))) {}
}

async fn disconnected(events: &mut papo::Events) -> papo::Disconnected {
    loop {
        match events.next().await {
            Some(papo::Event::Disconnected(disconnected)) => return disconnected,
            Some(_) => {}
            None => panic!("driver stopped before reporting the disconnection"),
        }
    }
}

#[tokio::test(start_paused = true)]
async fn send_before_welcome_fails_if_registration_disconnects() {
    let (client, _events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo-wire");
    assert_eq!(wire.next_sent().await, "USER papo-wire 0 * papo-wire");

    let send = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .send_raw(Line::privmsg("#papo", "release is ready for review"))
                .await
        }
    });
    tokio::task::yield_now().await;
    assert!(!client.state().is_connected());
    wire.server(":offline.test CAP * LS :").await;
    assert_eq!(wire.next_sent().await, "CAP END");
    assert!(
        tokio::time::timeout(Duration::from_millis(1), wire.next_sent())
            .await
            .is_err()
    );

    wire.close().await;
    assert!(matches!(
        send.await.unwrap(),
        Err(papo::Error::Disconnected(_))
    ));
}

#[tokio::test(start_paused = true)]
async fn explicit_reconnect_preserves_sends_queued_for_the_new_connection() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;

    client.reconnect().await.unwrap();
    let first = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .send_raw(Line::privmsg("#papo", "first queued update"))
                .await
        }
    });
    tokio::task::yield_now().await;
    let second = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .send_raw(Line::notice("#papo", "second queued update"))
                .await
        }
    });
    tokio::task::yield_now().await;

    assert_eq!(
        disconnected(&mut events).await.reconnect_in,
        Some(Duration::ZERO)
    );
    wire.close().await;
    assert!(!client.state().is_connected());
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo-wire");
    assert_eq!(wire.next_sent().await, "USER papo-wire 0 * papo-wire");
    wire.server(":offline.test CAP * LS :").await;
    assert_eq!(wire.next_sent().await, "CAP END");
    assert!(
        tokio::time::timeout(Duration::from_millis(1), wire.next_sent())
            .await
            .is_err()
    );

    wire.server(":offline.test 001 papo-wire :Welcome back to ErgoTest")
        .await;
    assert_eq!(wire.next_sent().await, "PRIVMSG #papo :first queued update");
    assert_eq!(wire.next_sent().await, "NOTICE #papo :second queued update");
    assert!(client.state().is_connected());
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
}
