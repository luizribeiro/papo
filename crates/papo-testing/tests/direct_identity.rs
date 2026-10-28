//! Highest-layer direct recipient identity regressions.

mod support {
    pub mod direct;
}

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    time::Duration,
};

use papo::Reconnect;
use papo_testing::wire;
use support::direct::{connected, next_active_event};

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
async fn direct_refs_use_logged_in_account_identity_for_equality_and_hashing() {
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
    ] {
        wire.server(line).await;
        assert!(matches!(
            next_active_event(&mut events).await,
            papo::Event::Joined(_)
        ));
    }
    let desktop = client.direct("alice").await.unwrap();
    let phone = client.direct("alice-phone").await.unwrap();
    assert_eq!(desktop, phone);

    let hash = |direct: &papo::DirectRef| {
        let mut hasher = DefaultHasher::new();
        direct.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(hash(&desktop), hash(&phone));
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
