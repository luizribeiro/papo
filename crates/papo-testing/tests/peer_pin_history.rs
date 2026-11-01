//! Highest-layer coverage for archived private-event peer pins.

mod support {
    pub mod direct;
    pub mod peer_pin;
}

use papo::{Event, Reconnect};
use papo_testing::wire;
use support::{direct::connected, peer_pin::join};

async fn history_page(
    client: &papo::Client,
    wire: &mut papo_testing::Wire,
    label: usize,
    nick: &str,
    account_tag: &str,
) -> papo::HistoryPage {
    let request = tokio::spawn({
        let client = client.clone();
        let nick = nick.to_owned();
        async move { client.history(nick).latest().await }
    });
    assert_eq!(
        wire.next_sent().await,
        format!("@label={label} CHATHISTORY LATEST {nick} * 100")
    );
    for line in [
        format!("@label={label} :offline.test BATCH +reply-{label} labeled-response"),
        format!(
            "@batch=reply-{label};draft/chathistory-end :offline.test BATCH +history-{label} chathistory {nick}"
        ),
        format!(
            "@batch=history-{label};time=2026-10-15T12:00:00.000Z;msgid=history-{label}{account_tag} :{nick}!~user@cloak.example PRIVMSG papo :archived review"
        ),
        format!(
            "@batch=history-{label};time=2026-10-15T12:00:01.000Z;msgid=history-tag-{label}{account_tag};+draft/example=ready :{nick}!~user@cloak.example TAGMSG papo"
        ),
        format!("@batch=reply-{label} :offline.test BATCH -history-{label}"),
        format!(":offline.test BATCH -reply-{label}"),
    ] {
        wire.server(line).await;
    }
    request.await.unwrap().unwrap()
}

#[tokio::test]
async fn history_uses_account_pins_without_reusing_live_session_pins() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("ircs://verified.example", "papo")
            .rate_limit(papo::RateLimit::unlimited())
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
        ":papo!p@localhost JOIN #review papo-account :Papo",
        ":alice!~alice@cloak.example JOIN #review alice-account :Alice",
        ":bob!~bob@users.example JOIN #review * :Bob",
    ] {
        join(&mut events, &mut wire, line).await;
    }
    let alice = client.direct("alice").await.unwrap().peer_key().clone();
    let bob = client.direct("bob").await.unwrap().peer_key().clone();

    let archived_alice =
        history_page(&client, &mut wire, 0, "alice", ";account=alice-account").await;
    let archived_bob = history_page(&client, &mut wire, 1, "bob", "").await;
    let alice_message = archived_alice.chat_messages().next().unwrap();
    let bob_message = archived_bob.chat_messages().next().unwrap();
    assert_eq!(alice_message.actor.peer_key(), Some(&alice));
    assert!(alice_message.actor.peer_key().unwrap().is_verified());
    assert_ne!(bob_message.actor.peer_key(), Some(&bob));
    assert_eq!(bob_message.actor.peer_key(), None);
    let alice_tag = archived_alice
        .items
        .iter()
        .find_map(|item| match item {
            Event::TagMsg(tag) => Some(tag),
            _ => None,
        })
        .unwrap();
    let bob_tag = archived_bob
        .items
        .iter()
        .find_map(|item| match item {
            Event::TagMsg(tag) => Some(tag),
            _ => None,
        })
        .unwrap();
    assert_eq!(alice_tag.actor.peer_key(), Some(&alice));
    assert_eq!(bob_tag.actor.peer_key(), None);
}
