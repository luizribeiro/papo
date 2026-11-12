//! Highest-layer direct recipient MONITOR regressions.

mod support {
    pub mod direct;
}

use std::time::Duration;

use papo::Reconnect;
use papo_testing::wire;
use support::direct::{connected, enable_monitor, next_active_event};

#[tokio::test]
async fn direct_ref_monitors_an_unshared_user_and_releases_the_entry() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 100).await;

    let request = tokio::spawn({
        let client = client.clone();
        async move { client.direct("alice").await }
    });
    assert_eq!(wire.next_sent().await, "MONITOR + alice");
    assert_eq!(wire.next_sent().await, "WHO alice %na");
    wire.server(":offline.test 354 papo alice alice-account")
        .await;
    wire.server(":offline.test 315 papo alice :End of WHO list")
        .await;
    let direct = request.await.unwrap().unwrap();
    let second_request = tokio::spawn({
        let client = client.clone();
        async move { client.direct("alice").await }
    });
    assert_eq!(wire.next_sent().await, "WHO alice %na");
    wire.server(":offline.test 354 papo alice alice-account")
        .await;
    wire.server(":offline.test 315 papo alice :End of WHO list")
        .await;
    let second = second_request.await.unwrap().unwrap();

    wire.server(":alice!~alice@cloak.example NICK alicia").await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::NickChanged(_)
    ));
    assert_eq!(wire.next_sent().await, "MONITOR - alice");
    assert_eq!(wire.next_sent().await, "MONITOR + alicia");
    client.action(&direct, "waves").await.unwrap();
    assert_eq!(
        wire.next_sent().await,
        "PRIVMSG alicia :\u{1}ACTION waves\u{1}"
    );
    drop(direct);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
    drop(second);
    assert_eq!(wire.next_sent().await, "MONITOR - alicia");
}

#[tokio::test]
async fn direct_ref_reopens_after_monitored_peer_returns() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 100).await;

    let first = tokio::spawn({
        let client = client.clone();
        async move { client.direct("alice").await }
    });
    assert_eq!(wire.next_sent().await, "MONITOR + alice");
    assert_eq!(wire.next_sent().await, "WHO alice %na");
    wire.server(":offline.test 354 papo alice alice-account")
        .await;
    wire.server(":offline.test 315 papo alice :End of WHO list")
        .await;
    let first = first.await.unwrap().unwrap();

    wire.server(":offline.test 731 papo :alice").await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::MonitorOffline(_)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err(),
        "731 must not release the live direct reference"
    );
    wire.server(":offline.test 730 papo :alice!~alice@cloak.example")
        .await;
    let online = tokio::time::timeout(Duration::from_secs(1), next_active_event(&mut events))
        .await
        .expect("returning monitored peer emits MonitorOnline");
    assert!(matches!(online, papo::Event::MonitorOnline(_)));

    let returned = tokio::spawn({
        let client = client.clone();
        async move { client.direct("alice").await }
    });
    let line = tokio::time::timeout(Duration::from_secs(1), wire.next_sent())
        .await
        .expect("returning peer starts a fresh WHO query");
    assert_eq!(line, "WHO alice %na");
    wire.server(":offline.test 354 papo alice alice-account")
        .await;
    wire.server(":offline.test 315 papo alice :End of WHO list")
        .await;
    let returned = returned.await.unwrap().unwrap();

    assert!(matches!(
        first.peer_key().check(returned.peer_key()),
        papo::PeerKeyCheck::Match | papo::PeerKeyCheck::Upgrade(_)
    ));
}

#[tokio::test]
async fn cancelled_direct_request_releases_monitor_exactly_once() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 100).await;

    let request = tokio::spawn(async move { client.direct("alice").await });
    assert_eq!(wire.next_sent().await, "MONITOR + alice");
    assert_eq!(wire.next_sent().await, "WHO alice %na");
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert_eq!(wire.next_sent().await, "MONITOR - alice");

    wire.server(":offline.test 730 papo :alice!~alice@cloak.example")
        .await;
    wire.server(":offline.test 731 papo :alice").await;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err(),
        "late monitor replies must not release the cancelled request again"
    );
}

#[tokio::test]
async fn who_resolved_direct_refs_share_monitor_until_both_drop() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 100).await;

    let first = tokio::spawn({
        let client = client.clone();
        async move { client.direct("alice").await }
    });
    assert_eq!(wire.next_sent().await, "MONITOR + alice");
    assert_eq!(wire.next_sent().await, "WHO alice %na");
    wire.server(":offline.test 354 papo alice alice-account")
        .await;
    wire.server(":offline.test 315 papo alice :End of WHO list")
        .await;
    let first = first.await.unwrap().unwrap();

    let second = tokio::spawn({
        let client = client.clone();
        async move { client.direct("alice").await }
    });
    assert_eq!(wire.next_sent().await, "WHO alice %na");
    wire.server(":offline.test 354 papo alice alice-account")
        .await;
    wire.server(":offline.test 315 papo alice :End of WHO list")
        .await;
    let second = second.await.unwrap().unwrap();

    wire.server(":offline.test 731 papo :alice").await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::MonitorOffline(_)
    ));
    wire.server(":offline.test 730 papo :alice!~alice@cloak.example")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::MonitorOnline(_)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err(),
        "presence changes must not release either direct reference"
    );

    drop(first);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err(),
        "the first reference must retain the shared monitor entry"
    );
    drop(second);
    assert_eq!(wire.next_sent().await, "MONITOR - alice");
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err(),
        "the shared monitor entry must be released exactly once"
    );
}

#[tokio::test]
async fn direct_ref_reports_a_full_monitor_list_without_failing_open() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 1).await;
    let _watch = client.query("bob").unwrap().watch();
    assert_eq!(wire.next_sent().await, "MONITOR + bob");

    assert!(matches!(
        client.direct("alice").await,
        Err(papo::Error::MonitorFull { limit: 1, .. })
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn direct_ref_refuses_when_who_finds_no_user_and_releases_monitor() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 100).await;

    let request = tokio::spawn(async move { client.direct("missing-user").await });
    assert_eq!(wire.next_sent().await, "MONITOR + missing-user");
    assert_eq!(wire.next_sent().await, "WHO missing-user %na");
    wire.server(":offline.test 315 papo missing-user :End of WHO list")
        .await;
    assert!(matches!(
        request.await.unwrap(),
        Err(papo::Error::TargetGone)
    ));
    assert_eq!(wire.next_sent().await, "MONITOR - missing-user");
}
