//! Highest-layer direct-message recipient identity regressions.

mod support {
    pub mod direct;
}

use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    time::Duration,
};

use papo::Reconnect;
use papo_testing::wire;
use support::direct::{connected, next_active_event};

fn handle_hash(user: &papo::UserHandle) -> u64 {
    let mut hasher = DefaultHasher::new();
    user.hash(&mut hasher);
    hasher.finish()
}

#[tokio::test]
#[allow(clippy::mutable_key_type)]
async fn learning_an_account_keeps_user_handle_identity() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-notify"]).await;
    connected(&mut events).await;
    for line in [
        ":papo!p@localhost JOIN #review",
        ":alice!~alice@cloak.example JOIN #review",
    ] {
        wire.server(line).await;
        assert!(matches!(
            next_active_event(&mut events).await,
            papo::Event::Joined(_)
        ));
    }
    let before_account = client.query("alice").unwrap();

    wire.server(":alice!~alice@cloak.example ACCOUNT alice-account")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::AccountChanged(_)
    ));
    let after_account = client.query("alice").unwrap();

    assert_eq!(before_account, after_account);
    assert_eq!(handle_hash(&before_account), handle_hash(&after_account));
    let by_recipient = HashMap::from([
        (before_account.clone(), "unknown"),
        (after_account.clone(), "logged in"),
    ]);
    let recipients = HashSet::from([before_account, after_account]);
    assert_eq!(by_recipient.len(), 1);
    assert_eq!(recipients.len(), 1);
}

#[tokio::test]
async fn user_handle_follows_rename_and_refuses_a_nickname_takeover() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    for line in [
        ":papo!p@localhost JOIN #review",
        ":alice!~alice@cloak.example JOIN #review",
        ":mallory!~mallory@users.example JOIN #review",
    ] {
        wire.server(line).await;
        assert!(matches!(
            next_active_event(&mut events).await,
            papo::Event::Joined(_)
        ));
    }
    let alice = client.query("alice").unwrap();

    wire.server(":alice!~alice@cloak.example NICK alicia").await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::NickChanged(_)
    ));
    let renamed = client.query("alicia").unwrap();
    assert_eq!(alice, renamed);
    assert_eq!(handle_hash(&alice), handle_hash(&renamed));
    client.privmsg(&alice, "release approved").await.unwrap();
    assert_eq!(wire.next_sent().await, "PRIVMSG alicia :release approved");

    wire.server(":alicia!~alice@cloak.example QUIT :Client Quit")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::Quit(_)
    ));
    wire.server(":mallory!~mallory@users.example NICK alicia")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::NickChanged(_)
    ));
    let replacement = client.query("alicia").unwrap();
    assert_ne!(alice, replacement);
    assert!(matches!(
        client.notice(&alice, "classified").await,
        Err(papo::Error::TargetGone)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn user_handles_identify_users_not_accounts() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["extended-join"]).await;
    connected(&mut events).await;
    for line in [
        ":papo!p@localhost JOIN #review papo-account :Papo",
        ":alice!~alice@cloak.example JOIN #review alice-account :Alice Desktop",
        ":alice-phone!~alice@phone.example JOIN #review alice-account :Alice Phone",
        ":mallory!~mallory@users.example JOIN #review mallory-account :Mallory",
        ":bob!~bob@users.example JOIN #review * :Bob",
        ":carol!~carol@users.example JOIN #review * :Carol",
    ] {
        wire.server(line).await;
        assert!(matches!(
            next_active_event(&mut events).await,
            papo::Event::Joined(_)
        ));
    }
    let desktop = client.query("alice").unwrap();
    let phone = client.query("alice-phone").unwrap();
    let mallory = client.query("mallory").unwrap();
    let bob = client.query("bob").unwrap();
    let carol = client.query("carol").unwrap();
    assert_ne!(desktop, phone);
    assert_ne!(desktop, mallory);
    assert_eq!(bob, bob.clone());
    assert_ne!(bob, carol);
    assert_eq!(desktop.to_string(), "alice");
    let debug = format!("{desktop:?}");
    assert!(debug.starts_with("UserHandle { nick: Nick"));
    assert!(debug.contains("\"alice\""));

    assert_ne!(handle_hash(&desktop), handle_hash(&phone));
    assert_ne!(handle_hash(&desktop), handle_hash(&mallory));
    assert_ne!(handle_hash(&bob), handle_hash(&carol));
    client.privmsg(&desktop, "desktop review").await.unwrap();
    assert_eq!(wire.next_sent().await, "PRIVMSG alice :desktop review");
}

#[tokio::test]
async fn nick_text_is_not_a_message_target() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;

    assert!(matches!(
        client.privmsg("alice", "unguarded").await,
        Err(papo::Error::InvalidInput(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}
