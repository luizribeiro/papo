//! Highest-layer untracked private reply regressions.

mod support {
    pub mod direct;
}

use std::time::Duration;

use papo::Reconnect;
use papo_testing::wire;
use support::direct::{connected, enable_monitor, next_active_event};

fn who_label(line: &str) -> String {
    let tagged = line
        .strip_prefix("@label=")
        .expect("WHO request was not labeled");
    let (label, command) = tagged.split_once(' ').expect("label had no command");
    assert_eq!(command, "WHO alice %na");
    label.to_owned()
}

async fn start_concurrent_replies(
    client: &papo::Client,
    message: &papo::Message,
    wire: &mut papo_testing::Wire,
) -> (
    tokio::task::JoinHandle<Result<papo::Sent, papo::Error>>,
    tokio::task::JoinHandle<Result<papo::Sent, papo::Error>>,
    String,
    String,
) {
    let first = tokio::spawn({
        let client = client.clone();
        let message = message.clone();
        async move { client.reply(&message, "first approval").await }
    });
    assert_eq!(wire.next_sent().await, "MONITOR + alice");
    let first_label = who_label(&wire.next_sent().await);
    let second = tokio::spawn({
        let client = client.clone();
        let message = message.clone();
        async move { client.reply(&message, "second approval").await }
    });
    let second_label = who_label(&wire.next_sent().await);
    (first, second, first_label, second_label)
}

async fn answer_who(wire: &mut papo_testing::Wire, label: &str, account: &str) {
    wire.server(format!(
        "@label={label} :offline.test 354 papo alice {account}"
    ))
    .await;
    wire.server(format!(
        "@label={label} :offline.test 315 papo alice :End of WHO list"
    ))
    .await;
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

#[tokio::test]
async fn concurrent_guarded_sends_refuse_an_impostor_during_account_proof() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-tag", "labeled-response", "message-tags"])
        .await;
    connected(&mut events).await;
    enable_monitor(&mut events, &mut wire, 100).await;
    let message = private_message(&mut events, &mut wire).await;

    let (first, second, first_label, second_label) =
        start_concurrent_replies(&client, &message, &mut wire).await;
    answer_who(&mut wire, &first_label, "mallory-account").await;
    answer_who(&mut wire, &second_label, "mallory-account").await;

    assert!(matches!(first.await.unwrap(), Err(papo::Error::TargetGone)));
    assert!(matches!(
        second.await.unwrap(),
        Err(papo::Error::TargetGone)
    ));
    assert_eq!(wire.next_sent().await, "MONITOR - alice");
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}
