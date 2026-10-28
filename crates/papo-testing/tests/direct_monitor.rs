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
async fn direct_ref_reports_a_full_monitor_list_without_failing_open() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 1).await;
    client.monitor().add(["bob"]).await.unwrap();
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
