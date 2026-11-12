//! Highest-layer coverage for archived private-event peer pins.

mod support {
    pub mod direct;
    pub mod peer_pin;
}

use std::time::Duration;

use papo::{Error, Event, Reconnect};
use papo_testing::wire;
use support::{
    direct::connected,
    peer_pin::{assert_match, join},
};

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

    let archived_alice =
        history_page(&client, &mut wire, 0, "alice", ";account=alice-account").await;
    let archived_bob = history_page(&client, &mut wire, 1, "bob", "").await;
    let alice_message = archived_alice.chat_messages().next().unwrap();
    let bob_message = archived_bob.chat_messages().next().unwrap();
    let bob_actor = bob_message.actor.clone();
    assert_match(&alice, &alice_message.actor.key());
    assert!(alice_message.actor.key().is_verified());
    assert!(!bob_message.actor.key().is_verified());
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
    assert_match(&alice, &alice_tag.actor.key());
    assert!(!bob_tag.actor.key().is_verified());

    let sending = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .privmsg(bob_actor, "current holder must not receive this")
                .await
        }
    });
    let who = wire.next_sent().await;
    let label = who
        .strip_prefix("@label=")
        .and_then(|tagged| tagged.split_once(' '))
        .map(|(label, command)| {
            assert_eq!(command, "WHO bob %na");
            format!("@label={label} ")
        })
        .unwrap_or_default();
    if label.is_empty() {
        assert_eq!(who, "WHO bob %na");
    }
    wire.server(format!(
        "{label}:offline.test 354 papo bob current-bob-account"
    ))
    .await;
    wire.server(format!(
        "{label}:offline.test 315 papo bob :End of WHO list"
    ))
    .await;
    assert!(matches!(sending.await.unwrap(), Err(Error::TargetGone)));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn soju_history_actor_matching_a_shared_user_can_receive_a_reply() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "reader")
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
        ":reader!~u@mhqynk9w52dpq.irc JOIN #incident * :Papo Reader",
        ":writer!~u@mhqynk9w52dpq.irc JOIN #incident * :Papo Writer",
    ] {
        join(&mut events, &mut wire, line).await;
    }
    wire.server(":reader!~u@mhqynk9w52dpq.irc NICK rreader")
        .await;

    let request = tokio::spawn({
        let client = client.clone();
        async move { client.history("writer").latest().limit(10).await }
    });
    assert_eq!(
        wire.next_sent().await,
        "@label=0 CHATHISTORY LATEST writer * 10"
    );
    for line in [
        "@label=0 BATCH +reply labeled-response",
        "@batch=reply BATCH +history chathistory writer",
        "@time=2026-11-12T05:21:06.731Z;msgid=rtaqwb2dxm8u85ap38mvhqxy62;batch=history :writer!~u@mhqynk9w52dpq.irc PRIVMSG reader :can you send the incident report?",
        "@batch=reply BATCH -history",
        "BATCH -reply",
    ] {
        wire.server(line).await;
    }
    let page = request.await.unwrap().unwrap();
    let message = page.chat_messages().next().unwrap().clone();

    let reply = tokio::spawn(async move { client.reply(&message, "the report is attached").await });
    assert_eq!(
        wire.next_sent().await,
        "@+reply=rtaqwb2dxm8u85ap38mvhqxy62 PRIVMSG writer :the report is attached"
    );
    assert!(reply.await.unwrap().is_ok());
}
