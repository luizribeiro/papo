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
use support::direct::{connected, join, next_active_event};

fn handle_hash(user: &papo::UserRef) -> u64 {
    let mut hasher = DefaultHasher::new();
    user.hash(&mut hasher);
    hasher.finish()
}

#[tokio::test]
#[allow(clippy::mutable_key_type)]
async fn learning_an_account_keeps_user_handle_identity() {
    let (_client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-notify"]).await;
    connected(&mut events).await;
    join(&mut events, &mut wire, ":papo!p@localhost JOIN #review").await;
    let before_account = join(
        &mut events,
        &mut wire,
        ":alice!~alice@cloak.example JOIN #review",
    )
    .await;

    wire.server(":alice!~alice@cloak.example ACCOUNT alice-account")
        .await;
    let papo::Event::AccountChanged(after_account) = next_active_event(&mut events).await else {
        panic!("expected AccountChanged");
    };
    let after_account = after_account.actor;

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
    join(&mut events, &mut wire, ":papo!p@localhost JOIN #review").await;
    let alice = join(
        &mut events,
        &mut wire,
        ":alice!~alice@cloak.example JOIN #review",
    )
    .await;
    let before_rename_hash = handle_hash(&alice);
    join(
        &mut events,
        &mut wire,
        ":mallory!~mallory@users.example JOIN #review",
    )
    .await;

    wire.server(":alice!~alice@cloak.example NICK alicia").await;
    let papo::Event::NickChanged(renamed) = next_active_event(&mut events).await else {
        panic!("expected NickChanged");
    };
    let renamed = renamed.actor;
    assert_eq!(alice, renamed);
    assert_eq!(before_rename_hash, handle_hash(&alice));
    assert_eq!(before_rename_hash, handle_hash(&renamed));
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
    let papo::Event::NickChanged(replacement) = next_active_event(&mut events).await else {
        panic!("expected NickChanged");
    };
    let replacement = replacement.actor;
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
async fn user_handles_from_separate_clients_are_distinct() {
    async fn observe_alice() -> (papo::Client, papo::UserRef) {
        let (client, mut events, mut wire) =
            wire(papo::Client::builder("irc://offline.test", "papo").reconnect(Reconnect::never()));
        wire.register(&[]).await;
        connected(&mut events).await;
        join(&mut events, &mut wire, ":papo!p@localhost JOIN #review").await;
        let alice = join(
            &mut events,
            &mut wire,
            ":alice!~alice@cloak.example JOIN #review",
        )
        .await;
        (client, alice)
    }

    let (first, alice) = observe_alice().await;
    let (second, same_nick) = observe_alice().await;
    assert_ne!(first.origin(), second.origin());
    assert_eq!(alice.nick().as_str(), same_nick.nick().as_str());
    assert_ne!(alice, same_nick);
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
    join(
        &mut events,
        &mut wire,
        ":papo!p@localhost JOIN #review papo-account :Papo",
    )
    .await;
    let desktop = join(
        &mut events,
        &mut wire,
        ":alice!~alice@cloak.example JOIN #review alice-account :Alice Desktop",
    )
    .await;
    let phone = join(
        &mut events,
        &mut wire,
        ":alice-phone!~alice@phone.example JOIN #review alice-account :Alice Phone",
    )
    .await;
    let mallory = join(
        &mut events,
        &mut wire,
        ":mallory!~mallory@users.example JOIN #review mallory-account :Mallory",
    )
    .await;
    let bob = join(
        &mut events,
        &mut wire,
        ":bob!~bob@users.example JOIN #review * :Bob",
    )
    .await;
    let carol = join(
        &mut events,
        &mut wire,
        ":carol!~carol@users.example JOIN #review * :Carol",
    )
    .await;
    assert_ne!(desktop, phone);
    assert_ne!(desktop, mallory);
    assert_eq!(bob, bob.clone());
    assert_ne!(bob, carol);
    assert_eq!(desktop.to_string(), "alice");
    let debug = format!("{desktop:?}");
    assert!(debug.starts_with("UserRef { nick: Nick"));
    assert!(debug.contains("\"alice\""));

    assert_ne!(handle_hash(&desktop), handle_hash(&phone));
    assert_ne!(handle_hash(&desktop), handle_hash(&mallory));
    assert_ne!(handle_hash(&bob), handle_hash(&carol));
    client.privmsg(&desktop, "desktop review").await.unwrap();
    assert_eq!(wire.next_sent().await, "PRIVMSG alice :desktop review");
}

#[tokio::test]
async fn text_targets_route_to_nicknames_and_channels() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;

    for target in ["alice", "#review"] {
        client.privmsg(target, "release approved").await.unwrap();
        assert_eq!(
            wire.next_sent().await,
            format!("PRIVMSG {target} :release approved")
        );
        client.notice(target, "review received").await.unwrap();
        assert_eq!(
            wire.next_sent().await,
            format!("NOTICE {target} :review received")
        );
    }
}

#[tokio::test]
async fn departed_handle_never_reaches_another_nick_on_the_same_account() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-tag", "extended-join"]).await;
    connected(&mut events).await;
    join(
        &mut events,
        &mut wire,
        ":papo!~papo@localhost JOIN #release * :Papo",
    )
    .await;
    let alice = join(
        &mut events,
        &mut wire,
        ":alice!~alice@cloak.example JOIN #release alice-account :Alice",
    )
    .await;
    join(
        &mut events,
        &mut wire,
        ":relaybot!~relay@bots.example JOIN #release alice-account :Shared relay bot",
    )
    .await;

    wire.server(":alice!~alice@cloak.example PART #release :bye")
        .await;
    while !matches!(next_active_event(&mut events).await, papo::Event::Parted(_)) {}
    let sent = tokio::time::timeout(
        Duration::from_secs(1),
        client.privmsg(&alice, "alice only: the deploy token is hunter2"),
    )
    .await
    .unwrap();
    assert!(matches!(sent, Err(papo::Error::TargetGone)), "{sent:?}");
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}
