//! Highest-layer untracked private reply regressions.

mod support {
    pub mod direct;
}

use std::time::Duration;

use papo::Reconnect;
use papo_testing::wire;
use support::direct::{connected, next_active_event};

type ReplyTask = tokio::task::JoinHandle<Result<papo::Sent, papo::Error>>;

fn start_concurrent_replies(
    client: &papo::Client,
    message: &papo::Message,
) -> (ReplyTask, ReplyTask) {
    let first = tokio::spawn({
        let client = client.clone();
        let message = message.clone();
        async move { client.reply(&message, "first approval").await }
    });
    let second = tokio::spawn({
        let client = client.clone();
        let message = message.clone();
        async move { client.reply(&message, "second approval").await }
    });
    (first, second)
}

async fn private_message(
    events: &mut papo::Events,
    wire: &mut papo_testing::Wire,
) -> papo::Message {
    wire.server(
        "@account=alice-account;msgid=ErgoGuardRace :alice!~alice@cloak.example PRIVMSG papo :release approval",
    )
    .await;
    let papo::Event::Message(message) = next_active_event(events).await else {
        panic!("expected private message")
    };
    message
}

#[tokio::test]
async fn private_reply_uses_event_proof_without_who() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-tag"]).await;
    connected(&mut events).await;
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
    assert_eq!(wire.next_sent().await, "PRIVMSG alice approved");
    assert!(reply.await.unwrap().is_ok());
}

#[tokio::test]
async fn private_reply_refuses_a_takeover_without_writing() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-tag"]).await;
    connected(&mut events).await;
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
    assert!(matches!(reply.await.unwrap(), Err(papo::Error::TargetGone)));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn concurrent_event_replies_refuse_an_impostor() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-tag", "labeled-response", "message-tags"])
        .await;
    connected(&mut events).await;
    let message = private_message(&mut events, &mut wire).await;
    wire.server(":alice!~alice@cloak.example QUIT :Client Quit")
        .await;
    _ = next_active_event(&mut events).await;
    wire.server(":mallory!~mallory@users.example NICK alice")
        .await;
    _ = next_active_event(&mut events).await;

    let (first, second) = start_concurrent_replies(&client, &message);

    assert!(matches!(first.await.unwrap(), Err(papo::Error::TargetGone)));
    assert!(matches!(
        second.await.unwrap(),
        Err(papo::Error::TargetGone)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn concurrent_guarded_sends_refuse_a_takeover_across_reconnect() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(
                Reconnect::exponential()
                    .max_delay(Duration::from_secs(1))
                    .max_attempts(2),
            ),
    );
    let caps = ["account-tag", "labeled-response", "message-tags"];
    wire.register(&caps).await;
    connected(&mut events).await;
    let message = private_message(&mut events, &mut wire).await;

    wire.close().await;
    let delay = loop {
        let Some(event) = events.next().await.map(|(event, _)| event) else {
            panic!("driver stopped before reconnecting")
        };
        if let papo::Event::Disconnected(disconnected) = event {
            break disconnected
                .reconnect_in
                .expect("closed wire should reconnect");
        }
    };
    tokio::time::advance(delay).await;
    wire.register(&caps).await;
    connected(&mut events).await;
    let (first, second) = start_concurrent_replies(&client, &message);

    assert!(matches!(first.await.unwrap(), Err(papo::Error::TargetGone)));
    assert!(matches!(
        second.await.unwrap(),
        Err(papo::Error::TargetGone)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn concurrent_guarded_sends_reach_the_proven_recipient() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-tag", "labeled-response", "message-tags"])
        .await;
    connected(&mut events).await;
    let message = private_message(&mut events, &mut wire).await;

    let (first, second) = start_concurrent_replies(&client, &message);

    let mut sent = [wire.next_sent().await, wire.next_sent().await];
    sent.sort();
    assert_eq!(
        sent,
        [
            "@+reply=ErgoGuardRace PRIVMSG alice :first approval",
            "@+reply=ErgoGuardRace PRIVMSG alice :second approval"
        ]
    );
    assert!(first.await.unwrap().is_ok());
    assert!(second.await.unwrap().is_ok());
}
