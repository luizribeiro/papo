//! Highest-layer untracked private reply regressions.

mod support {
    pub mod direct;
}

use papo::Reconnect;
use papo_testing::wire;
use support::direct::{connected, enable_monitor, next_active_event};

#[tokio::test]
async fn untracked_private_reply_verifies_account_before_sending() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-tag"]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 100).await;
    wire.server(
        "@account=alice-account;msgid=ErgoGuard1 :alice!~alice@cloak.example PRIVMSG papo :release approval",
    )
    .await;
    let papo::Event::Message(message) = next_active_event(&mut events).await else {
        panic!("expected private message")
    };

    let reply = tokio::spawn({
        let client = client.clone();
        async move { client.reply(&message, "approved").await }
    });
    assert_eq!(wire.next_sent().await, "MONITOR + alice");
    assert_eq!(wire.next_sent().await, "WHO alice %na");
    wire.server(":offline.test 354 papo alice alice-account")
        .await;
    wire.server(":offline.test 315 papo alice :End of WHO list")
        .await;
    assert_eq!(wire.next_sent().await, "PRIVMSG alice approved");
    assert!(reply.await.unwrap().is_ok());
    assert_eq!(wire.next_sent().await, "MONITOR - alice");
}

#[tokio::test]
async fn untracked_private_reply_refuses_a_who_proven_takeover() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-tag"]).await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 100).await;
    wire.server(
        "@account=alice-account;msgid=ErgoGuard2 :alice!~alice@cloak.example PRIVMSG papo :release approval",
    )
    .await;
    let papo::Event::Message(message) = next_active_event(&mut events).await else {
        panic!("expected private message")
    };
    wire.server(":alice!~alice@cloak.example QUIT :Client Quit")
        .await;
    _ = next_active_event(&mut events).await;
    wire.server(":mallory!~mallory@users.example NICK alice")
        .await;
    _ = next_active_event(&mut events).await;

    let reply = tokio::spawn(async move { client.reply(&message, "classified").await });
    assert_eq!(wire.next_sent().await, "MONITOR + alice");
    assert_eq!(wire.next_sent().await, "WHO alice %na");
    wire.server(":offline.test 354 papo alice mallory-account")
        .await;
    wire.server(":offline.test 315 papo alice :End of WHO list")
        .await;
    assert!(matches!(reply.await.unwrap(), Err(papo::Error::TargetGone)));
    assert_eq!(wire.next_sent().await, "MONITOR - alice");
}
