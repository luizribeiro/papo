//! Checks that tags on history lines never borrow the account of the current user with that nick.

mod support {
    pub mod direct;
}

use papo::{AccountStatus, Client, Event, RateLimit, Reconnect};
use papo_testing::wire;
use support::direct::{connected, next_active_event};

#[tokio::test]
async fn archived_account_evidence_ignores_the_current_account() {
    let (client, mut events, mut wire) = wire(
        Client::builder("irc://offline.test", "papo")
            .rate_limit(RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "account-tag",
        "batch",
        "draft/chathistory",
        "extended-join",
        "labeled-response",
        "message-tags",
        "server-time",
    ])
    .await;
    connected(&mut events).await;
    for line in [
        ":papo!~papo@localhost JOIN #review * :Papo",
        ":alice!~alice@cloak.example JOIN #review alice-today :Alice",
        "@account=alice-today :alice!~alice@cloak.example PRIVMSG #review :Today's review",
    ] {
        wire.server(line).await;
    }
    let current = loop {
        if let Event::Message(message) = next_active_event(&mut events).await {
            break message;
        }
    };
    assert!(
        matches!(current.actor_account(), AccountStatus::LoggedIn(account) if account == "alice-today")
    );
    let history = tokio::spawn(async move { client.history("#review").latest().await });
    assert_eq!(
        wire.next_sent().await,
        "@label=0 CHATHISTORY LATEST #review * 100"
    );
    for line in [
        "@label=0 :offline.test BATCH +reply labeled-response",
        "@batch=reply;draft/chathistory-end :offline.test BATCH +h chathistory #review",
        "@batch=h;time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120050;+account=alice-today :alice!~alice@cloak.example PRIVMSG #review :Unauthenticated review",
        "@batch=h;time=2026-10-15T12:00:01.000Z;msgid=RfcBTzyxY120051;account=alice-before :alice!~alice@cloak.example PRIVMSG #review :Authenticated review",
        "@batch=h;time=2026-10-15T12:00:02.000Z;msgid=RfcBTzyxY120052;account=alice-before;+example/status=ready :alice!~alice@cloak.example TAGMSG #review",
        "@batch=reply :offline.test BATCH -h",
        ":offline.test BATCH -reply",
    ] {
        wire.server(line).await;
    }
    let history = history.await.unwrap().unwrap();
    let mut messages = history.chat_messages();
    assert_eq!(
        messages.next().unwrap().actor_account(),
        &AccountStatus::Unknown
    );
    assert!(matches!(
        messages.next().unwrap().actor_account(),
        AccountStatus::LoggedIn(account) if account == "alice-before"
    ));
    assert!(messages.next().is_none());
    let tag = history.items.iter().find_map(Event::as_tag_msg).unwrap();
    assert!(
        matches!(tag.actor_account(), AccountStatus::LoggedIn(account) if account == "alice-before")
    );
}
