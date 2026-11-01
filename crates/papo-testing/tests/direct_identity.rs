//! Highest-layer direct recipient identity regressions.

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

fn direct_hash(direct: &papo::DirectRef) -> u64 {
    let mut hasher = DefaultHasher::new();
    direct.hash(&mut hasher);
    hasher.finish()
}

#[tokio::test]
#[allow(clippy::mutable_key_type)]
async fn learning_an_account_keeps_direct_ref_identity() {
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
    let before_account = client.direct("alice").await.unwrap();

    wire.server(":alice!~alice@cloak.example ACCOUNT alice-account")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::AccountChanged(_)
    ));
    let after_account = client.direct("alice").await.unwrap();

    assert_eq!(before_account, after_account);
    assert_eq!(direct_hash(&before_account), direct_hash(&after_account));
    let by_recipient = HashMap::from([
        (before_account.clone(), "unknown"),
        (after_account.clone(), "logged in"),
    ]);
    let recipients = HashSet::from([before_account, after_account]);
    assert_eq!(by_recipient.len(), 1);
    assert_eq!(recipients.len(), 1);
}

#[tokio::test]
async fn direct_ref_follows_rename_and_refuses_a_nickname_takeover() {
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
    let direct = client.direct("alice").await.unwrap();

    wire.server(":alice!~alice@cloak.example NICK alicia").await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::NickChanged(_)
    ));
    let renamed = client.direct("alicia").await.unwrap();
    assert_eq!(direct, renamed);
    assert_eq!(direct_hash(&direct), direct_hash(&renamed));
    client.privmsg(&direct, "release approved").await.unwrap();
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
    let replacement = client.direct("alicia").await.unwrap();
    assert_ne!(direct, replacement);
    assert!(matches!(
        client.notice(&direct, "classified").await,
        Err(papo::Error::TargetGone)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn direct_refs_identify_users_not_accounts() {
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
    let desktop = client.direct("alice").await.unwrap();
    let phone = client.direct("alice-phone").await.unwrap();
    let mallory = client.direct("mallory").await.unwrap();
    let bob = client.direct("bob").await.unwrap();
    let carol = client.direct("carol").await.unwrap();
    assert_ne!(desktop, phone);
    assert_ne!(desktop, mallory);
    assert_eq!(bob, bob.clone());
    assert_ne!(bob, carol);
    assert_eq!(desktop.to_string(), "alice");
    let debug = format!("{desktop:?}");
    assert!(debug.starts_with("DirectRef { nick: Nick"));
    assert!(debug.contains("\"alice\""));

    assert_ne!(direct_hash(&desktop), direct_hash(&phone));
    assert_ne!(direct_hash(&desktop), direct_hash(&mallory));
    assert_ne!(direct_hash(&bob), direct_hash(&carol));
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
