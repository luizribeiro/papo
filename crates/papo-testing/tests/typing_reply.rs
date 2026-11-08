//! Highest-layer retained typing return-address regressions.

use papo::{Event, MessageRef, Reconnect};
use papo_testing::wire;

fn reply_reference(expected: &str, event: Event) -> MessageRef {
    match (expected, event) {
        ("typing", Event::TypingChanged(event)) => event.return_address(),
        ("reaction", Event::Reacted(event)) => event.return_address(),
        ("redaction", Event::Redacted(event)) => event.return_address(),
        (_, event) => panic!("unexpected reply event: {event:?}"),
    }
}

async fn next_active_event(events: &mut papo::Events) -> Event {
    loop {
        let event = events
            .next()
            .await
            .map(|(event, _)| event)
            .expect("client stopped");
        if !matches!(event, Event::Other(_)) {
            return event;
        }
    }
}

#[tokio::test]
async fn retained_typers_reply_to_private_and_ops_only_audiences() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["message-tags"]).await;
    while !matches!(next_active_event(&mut events).await, Event::Connected(_)) {}
    wire.server(":offline.test 005 papo-wire STATUSMSG=@+ :are supported")
        .await;
    for line in [
        ":papo-wire!p@localhost JOIN #ops",
        ":alice!~a@cloak.example JOIN #ops",
    ] {
        wire.server(line).await;
        assert!(matches!(
            next_active_event(&mut events).await,
            Event::Joined(_)
        ));
    }

    wire.server("@+typing=active :alice!~a@cloak.example TAGMSG papo-wire")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        Event::TypingChanged(_)
    ));
    let private = {
        let state = client.snapshot();
        let conversation = state.name_rules().unwrap().conversation("alice").unwrap();
        state.typing(conversation).next().unwrap().return_address()
    };
    client.reply(private, "private pong").await.unwrap();
    assert_eq!(wire.next_sent().await, "PRIVMSG alice :private pong");

    wire.server("@+typing=paused :alice!~a@cloak.example TAGMSG @#ops")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        Event::TypingChanged(_)
    ));
    let operators = {
        let state = client.snapshot();
        let conversation = state.name_rules().unwrap().conversation("#ops").unwrap();
        state.typing(conversation).next().unwrap().return_address()
    };
    client.reply(operators, "ops pong").await.unwrap();
    assert_eq!(wire.next_sent().await, "PRIVMSG @#ops :ops pong");
}

#[tokio::test]
async fn core_reply_events_guard_only_private_recipients() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "message-tags",
        "draft/message-redaction",
        "draft/react",
        "typing",
    ])
    .await;
    while !matches!(next_active_event(&mut events).await, Event::Connected(_)) {}
    for line in [
        ":papo-wire!p@localhost JOIN #review",
        ":alice!~alice@cloak.example JOIN #review",
        ":mallory!~mallory@users.example JOIN #review",
    ] {
        wire.server(line).await;
        assert!(matches!(
            next_active_event(&mut events).await,
            Event::Joined(_)
        ));
    }

    let cases = [
        (
            "typing",
            "@+typing=active :alice!~alice@cloak.example TAGMSG papo-wire",
            "@+typing=active :alice!~alice@cloak.example TAGMSG #review",
        ),
        (
            "reaction",
            "@msgid=ErgoReply2;+draft/react=✅;+draft/reply=ErgoReply1 :alice!~alice@cloak.example TAGMSG papo-wire",
            "@msgid=ErgoReply4;+draft/react=✅;+draft/reply=ErgoReply3 :alice!~alice@cloak.example TAGMSG #review",
        ),
        (
            "redaction",
            "@msgid=ErgoReply6 :alice!~alice@cloak.example REDACT papo-wire ErgoReply5 :superseded",
            "@msgid=ErgoReply8 :alice!~alice@cloak.example REDACT #review ErgoReply7 :superseded",
        ),
    ];
    let mut references = Vec::new();
    for (name, private_line, channel_line) in cases {
        wire.server(private_line).await;
        let private = reply_reference(name, next_active_event(&mut events).await);
        assert_eq!(private.as_ref(), "alice", "{name}");
        client
            .reply(private.clone(), format!("{name} private reply"))
            .await
            .unwrap();
        assert!(
            wire.next_sent()
                .await
                .ends_with(&format!("PRIVMSG alice :{name} private reply")),
            "{name}"
        );

        wire.server(channel_line).await;
        let channel = reply_reference(name, next_active_event(&mut events).await);
        assert_eq!(channel.as_ref(), "#review", "{name}");
        references.push((name, private, channel));
    }

    wire.server(":alice!~alice@cloak.example QUIT :Client Quit")
        .await;
    while !matches!(next_active_event(&mut events).await, Event::Quit(_)) {}
    wire.server(":mallory!~mallory@users.example NICK alice")
        .await;
    while !matches!(next_active_event(&mut events).await, Event::NickChanged(_)) {}

    for (name, private, channel) in references {
        assert!(
            matches!(
                client.reply(private, "confidential follow-up").await,
                Err(papo::Error::TargetGone)
            ),
            "{name}"
        );
        client
            .reply(channel, format!("{name} channel reply"))
            .await
            .unwrap();
        assert!(
            wire.next_sent()
                .await
                .ends_with(&format!("PRIVMSG #review :{name} channel reply")),
            "{name}"
        );
    }
}
