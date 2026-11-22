//! Identity and routing coverage for archived messages.

mod support {
    pub mod direct;
}

use std::{future::IntoFuture as _, time::Duration};

use papo::{Error, Event, Reconnect};
use papo_testing::wire;
use support::direct::{connected, join, next_active_event};

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
async fn archived_private_events_preserve_accounts_and_reject_stale_actors() {
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
    let archived_alice =
        history_page(&client, &mut wire, 0, "alice", ";account=alice-account").await;
    let archived_bob = history_page(&client, &mut wire, 1, "bob", "").await;
    let alice_message = archived_alice.chat_messages().next().unwrap();
    let bob_message = archived_bob.chat_messages().next().unwrap();
    assert!(matches!(
        alice_message.actor_account(),
        papo::Account::LoggedIn(account) if account == "alice-account"
    ));
    assert_eq!(bob_message.actor_account(), &papo::Account::Unknown);
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
    assert_eq!(alice_tag.actor_account(), alice_message.actor_account());
    assert_eq!(bob_tag.actor_account(), &papo::Account::Unknown);

    for message in [alice_message, bob_message] {
        assert!(matches!(
            client
                .privmsg(&message.actor, "current holder must not receive this")
                .await,
            Err(Error::TargetGone)
        ));
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn soju_history_reply_uses_the_peer_across_our_rename() {
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

#[tokio::test]
async fn untagged_history_line_is_not_the_current_nick_holder() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("ircs://ergo.test", "papo")
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
    join(
        &mut events,
        &mut wire,
        ":papo!~u@papo.irc JOIN #release papo :Papo",
    )
    .await;
    let live = join(
        &mut events,
        &mut wire,
        ":writer!~u@tor-network.onion JOIN #release writer :Writer",
    )
    .await;

    let request = tokio::spawn({
        let client = client.clone();
        async move { client.history("#release").latest().await }
    });
    assert_eq!(
        wire.next_sent().await,
        "@label=0 CHATHISTORY LATEST #release * 100"
    );
    for line in [
        "@label=0 :ergo.test BATCH +reply labeled-response",
        "@batch=reply;draft/chathistory-end :ergo.test BATCH +h chathistory #release",
        "@batch=h;time=2026-09-01T08:00:00.000Z;msgid=ErgoOld1 :writer!~u@tor-network.onion PRIVMSG #release :I approve wiping the production database",
        "@batch=h;time=2026-11-16T09:30:00.000Z;msgid=ErgoNew2;account=writer :writer!~u@tor-network.onion PRIVMSG #release :the release is tagged",
        "@batch=h;time=2026-11-16T09:31:00.000Z;msgid=ErgoNew3;account=writer :writer!~u@tor-network.onion PRIVMSG #release :the notes are ready too",
        "@batch=reply :ergo.test BATCH -h",
        ":ergo.test BATCH -reply",
    ] {
        wire.server(line).await;
    }
    let page = request.await.unwrap().unwrap();
    let [untagged, tagged, next_tagged] =
        [0, 1, 2].map(|index| page.chat_messages().nth(index).unwrap().clone());

    assert_ne!(untagged.actor, live);
    assert_eq!(untagged.actor_account(), &papo::Account::Unknown);
    assert_ne!(tagged.actor, live);
    assert_ne!(tagged.actor, untagged.actor);
    assert_ne!(tagged.actor, next_tagged.actor);
    assert_eq!(tagged.actor, tagged.actor.clone());
    assert!(matches!(
        tagged.actor_account(),
        papo::Account::LoggedIn(account) if account == "writer"
    ));

    let sending =
        tokio::spawn(async move { client.privmsg(tagged.actor, "thanks, shipping it").await });
    assert_eq!(
        wire.next_sent().await,
        "PRIVMSG writer :thanks, shipping it"
    );
    assert!(sending.await.unwrap().is_ok());
}

#[tokio::test]
async fn logged_out_reply_to_our_archived_direct_message_never_reaches_us() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://ergo.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "account-tag",
        "batch",
        "draft/chathistory",
        "draft/message-redaction",
        "extended-join",
        "labeled-response",
        "message-tags",
        "server-time",
    ])
    .await;
    connected(&mut events).await;
    for line in [
        ":papo!~u@papo.irc JOIN #release * :Papo",
        ":alice!~u@alice.irc JOIN #release alice :Alice",
    ] {
        join(&mut events, &mut wire, line).await;
    }

    let request = tokio::spawn({
        let client = client.clone();
        async move { client.history("alice").latest().limit(10).await }
    });
    assert_eq!(
        wire.next_sent().await,
        "@label=0 CHATHISTORY LATEST alice * 10"
    );
    for line in [
        "@label=0 :ergo.test BATCH +reply labeled-response",
        "@batch=reply;draft/chathistory-end :ergo.test BATCH +h chathistory alice",
        "@batch=h;time=2026-11-01T09:00:00.000Z;msgid=6e7q9yyfrx3nwsw2i6yjzmzxyc :papo!~u@papo.irc PRIVMSG alice :I will send the token once you confirm",
        "@batch=h;time=2026-11-01T09:01:00.000Z;msgid=pwcbkkexfp2uqmk5ri5x5b5bya;account=alice :alice!~u@alice.irc PRIVMSG papo :confirmed, send it",
        "@batch=reply :ergo.test BATCH -h",
        ":ergo.test BATCH -reply",
    ] {
        wire.server(line).await;
    }
    let page = request.await.unwrap().unwrap();
    let [ours, theirs] = [0, 1].map(|index| page.chat_messages().nth(index).unwrap().clone());
    for message in [&ours, &theirs] {
        let papo::Conversation::Private(peer) = message.conversation() else {
            panic!("expected a direct conversation")
        };
        assert_eq!(peer.nick(), "alice");
    }

    for (message, msgid) in [
        (ours, "6e7q9yyfrx3nwsw2i6yjzmzxyc"),
        (theirs.clone(), "pwcbkkexfp2uqmk5ri5x5b5bya"),
    ] {
        let reply = tokio::spawn(client.reply(&message, "the release is ready").into_future());
        assert_eq!(
            wire.next_sent().await,
            format!("@+reply={msgid} PRIVMSG alice :the release is ready")
        );
        assert!(reply.await.unwrap().is_ok());
    }

    let msgid = "pwcbkkexfp2uqmk5ri5x5b5bya";
    for (label, request, tag) in [
        (3, client.react(&theirs, "✅"), "+draft/react"),
        (4, client.unreact(&theirs, "✅"), "+draft/unreact"),
    ] {
        let result = tokio::spawn(request);
        assert_eq!(
            wire.next_sent().await,
            format!("@+reply={msgid};{tag}=✅;label={label} TAGMSG alice")
        );
        wire.server(format!("@label={label} :ergo.test ACK")).await;
        assert!(result.await.unwrap().is_ok());
    }
    let redaction = tokio::spawn(client.redact(&theirs, "superseded"));
    assert_eq!(
        wire.next_sent().await,
        format!("@label=5 REDACT alice {msgid} superseded")
    );
    wire.server("@label=5 :ergo.test ACK").await;
    assert!(redaction.await.unwrap().is_ok());

    wire.server(":papo!~u@papo.irc NICK ALICE").await;
    while !matches!(next_active_event(&mut events).await, Event::NickChanged(change) if change.new == "ALICE")
    {
    }
    assert!(matches!(
        client.reply(&theirs, "the release is ready").await,
        Err(Error::TargetGone)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}
