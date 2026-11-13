//! Highest-layer direct recipient MONITOR regressions.

mod support {
    pub mod direct;
}

use std::time::Duration;

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

async fn answer_who(wire: &mut Wire, row: Option<&str>) {
    assert_eq!(wire.next_sent().await, "WHO alice %na");
    if let Some(row) = row {
        wire.server(row).await;
    }
    wire.server(":offline.test 315 papo alice :End of WHO list")
        .await;
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

    let alice = client.query("alice").unwrap();
    assert_silent(&mut wire, "a query handle spends no MONITOR slot").await;
    let first = alice.watch();
    assert_eq!(wire.next_sent().await, "MONITOR + alice");
    let second = client.query("alice").unwrap().watch();
    assert_silent(&mut wire, "a second watch shares the entry").await;

    let action = tokio::spawn({
        let client = client.clone();
        let alice = alice.clone();
        async move { client.action(&alice, "waves").await }
    });
    answer_who(
        &mut wire,
        Some(":offline.test 354 papo alice alice-account"),
    )
    .await;
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

    let watch = client.query("alice").unwrap().watch();
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

#[tokio::test]
async fn cancelled_query_send_spends_no_monitor_slot() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 100).await;

    let alice = client.query("alice").unwrap();
    let request = tokio::spawn(async move { client.privmsg(&alice, "release approved").await });
    assert_eq!(wire.next_sent().await, "WHO alice %na");
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    wire.server(":offline.test 354 papo alice alice-account")
        .await;
    wire.server(":offline.test 315 papo alice :End of WHO list")
        .await;
    assert_silent(
        &mut wire,
        "a cancelled send sends neither MONITOR nor PRIVMSG",
    )
    .await;
}

#[tokio::test]
async fn query_send_does_not_fail_when_the_monitor_list_is_full() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 1).await;
    let _bob = client.query("bob").unwrap().watch();
    assert_eq!(wire.next_sent().await, "MONITOR + bob");

    let alice = client.query("alice").unwrap();
    let watch = alice.watch();
    let request = tokio::spawn({
        let client = client.clone();
        async move { client.privmsg(&alice, "release approved").await }
    });
    answer_who(
        &mut wire,
        Some(":offline.test 354 papo alice alice-account"),
    )
    .await;
    assert_eq!(wire.next_sent().await, "PRIVMSG alice :release approved");
    assert!(request.await.unwrap().is_ok());
    assert!(!watch.is_live());
}

#[tokio::test]
async fn query_send_refuses_when_who_finds_no_user() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 100).await;

    let alice = client.query("alice").unwrap();
    let request = tokio::spawn(async move { client.notice(&alice, "release approved").await });
    answer_who(&mut wire, None).await;
    assert!(matches!(
        request.await.unwrap(),
        Err(papo::Error::TargetGone)
    ));
    assert_silent(&mut wire, "an unprovable recipient gets no NOTICE").await;
}
