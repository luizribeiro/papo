//! Highest-layer direct recipient MONITOR regressions.

mod support {
    pub mod direct;
}

use std::{future::IntoFuture as _, time::Duration};

use papo::Reconnect;
use papo_testing::{Wire, wire};
use support::direct::{connected, enable_monitor, next_active_event};

async fn assert_silent(wire: &mut Wire, reason: &str) {
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err(),
        "{reason}"
    );
}

#[tokio::test]
async fn watch_monitors_an_unshared_user_and_releases_the_entry() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 100).await;

    let first = client.watch("alice");
    assert_eq!(wire.next_sent().await, "MONITOR + alice");
    let second = client.watch("alice");
    assert_silent(&mut wire, "a second watch shares the entry").await;

    let action = tokio::spawn(client.action("alice", "waves").into_future());
    assert_eq!(
        wire.next_sent().await,
        "PRIVMSG alice :\u{1}ACTION waves\u{1}"
    );
    action.await.unwrap().unwrap();
    drop(first);
    assert_silent(&mut wire, "the second watch retains the entry").await;
    drop(second);
    assert_eq!(wire.next_sent().await, "MONITOR - alice");
    assert_silent(&mut wire, "the shared entry is released exactly once").await;
}

#[tokio::test]
async fn watch_is_kept_while_peer_is_offline() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 100).await;

    let watch = client.watch("alice");
    assert_eq!(wire.next_sent().await, "MONITOR + alice");
    wire.server(":offline.test 731 papo :alice").await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::MonitorOffline(_)
    ));
    assert_silent(&mut wire, "731 must not release the live watch").await;
    wire.server(":offline.test 730 papo :alice!~alice@cloak.example")
        .await;
    let online = tokio::time::timeout(Duration::from_secs(1), next_active_event(&mut events))
        .await
        .expect("returning monitored peer emits MonitorOnline");
    assert!(matches!(online, papo::Event::MonitorOnline(_)));

    drop(watch);
    assert_eq!(wire.next_sent().await, "MONITOR - alice");
}
