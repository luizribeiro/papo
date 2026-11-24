//! Reply identities on assembled messages.

mod support {
    pub mod direct;
    pub mod message_peer;
}

use support::message_peer::{FIRST, assert_refused, message, start};

const MULTILINE: &str = "draft/multiline=max-bytes=4096,max-lines=100";

const MULTILINE_FROM_MALLORY: &[&str] = &[
    "@msgid=ErgoDm2;account=mallory :alice!~mallory@tor-exit.example BATCH +ml1 draft/multiline papo",
    "@batch=ml1 :alice!~mallory@tor-exit.example PRIVMSG papo :hello?",
    "@batch=ml1 :alice!~mallory@tor-exit.example PRIVMSG papo :are you there?",
    ":alice!~mallory@tor-exit.example BATCH -ml1",
];

#[tokio::test]
async fn multiline_message_from_another_host_ends_a_message_only_reference() {
    let (client, mut events, mut wire) = start(&["batch", "message-tags", MULTILINE]).await;
    let first = message(&mut events, &mut wire, &[FIRST]).await;
    let second = message(&mut events, &mut wire, MULTILINE_FROM_MALLORY).await;
    assert_eq!(second.text, "hello?\nare you there?");
    assert_refused(client, &mut wire, first).await;
}

/// The first message carries `account=alice`; the multiline one says `account=mallory`.
#[tokio::test]
async fn multiline_message_from_another_account_ends_the_reference() {
    let (client, mut events, mut wire) =
        start(&["account-tag", "batch", "message-tags", MULTILINE]).await;
    let first = message(
        &mut events,
        &mut wire,
        &["@msgid=ErgoDm1;account=alice :alice!~alice@cloak-7f3a.example PRIVMSG papo :is the release out?"],
    )
    .await;
    let same_host = MULTILINE_FROM_MALLORY
        .iter()
        .map(|line| line.replace("~mallory@tor-exit.example", "~alice@cloak-7f3a.example"))
        .collect::<Vec<_>>();
    let lines = same_host.iter().map(String::as_str).collect::<Vec<_>>();
    let second = message(&mut events, &mut wire, &lines).await;
    assert!(matches!(
        second.actor_account(),
        papo::AccountStatus::LoggedIn(account) if account == "mallory"
    ));
    assert_refused(client, &mut wire, first).await;
}

/// `UserRef` equality: "equal only for the same tracked user".
#[tokio::test]
async fn multiline_sender_on_another_host_is_not_the_earlier_sender() {
    let (_client, mut events, mut wire) = start(&["batch", "message-tags", MULTILINE]).await;
    let first = message(&mut events, &mut wire, &[FIRST]).await;
    let second = message(&mut events, &mut wire, MULTILINE_FROM_MALLORY).await;
    assert_ne!(first.actor, second.actor);
}

#[tokio::test]
async fn a_first_multiline_private_message_can_be_answered() {
    let (client, mut events, mut wire) = start(&["batch", "message-tags", MULTILINE]).await;
    let first = message(
        &mut events,
        &mut wire,
        &[
            "@msgid=ErgoDm1 :alice!~alice@cloak-7f3a.example BATCH +ml1 draft/multiline papo",
            "@batch=ml1 :alice!~alice@cloak-7f3a.example PRIVMSG papo :can you send me",
            "@batch=ml1 :alice!~alice@cloak-7f3a.example PRIVMSG papo :the release details?",
            ":alice!~alice@cloak-7f3a.example BATCH -ml1",
        ],
    )
    .await;
    client.reply(&first, "which one?").await.unwrap();
    assert_eq!(
        wire.next_sent().await,
        "@+reply=ErgoDm1 PRIVMSG alice :which one?"
    );
}
