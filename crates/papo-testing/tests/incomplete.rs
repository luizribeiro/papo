//! Offline wire coverage for cut-off multiline delivery.

use papo::{Event, Incomplete, Reconnect};
use papo_testing::wire;

#[tokio::test(start_paused = true)]
async fn dropped_connection_delivers_the_partial_message_before_disconnect() {
    let (_client, mut events, mut wire) = wire(
        papo::Client::builder("irc://hostile.offline.test", "papo-wire")
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "message-tags",
        "batch",
        "draft/multiline=max-bytes=4096,max-lines=100",
    ])
    .await;
    while !matches!(events.next().await, Some((Event::Connected(_), _))) {}

    for line in [
        "@msgid=RKi9Cw;time=2026-10-15T12:00:01.000Z :alice!~a@cloak BATCH +deploy draft/multiline #papo",
        "@batch=deploy :alice!~a@cloak PRIVMSG #papo :deploying release",
        "@batch=deploy :alice!~a@cloak PRIVMSG #papo :waiting for health checks",
    ] {
        wire.server(line).await;
    }
    wire.close().await;

    let (event, state) = events
        .next()
        .await
        .expect("partial message should precede disconnection");
    let Event::Incomplete(Incomplete::Message(message)) = event else {
        panic!("expected an incomplete message, got {event:?}")
    };
    assert_eq!(message.text, "deploying release\nwaiting for health checks");
    assert_eq!(message.conversation(), "#papo");
    assert_eq!(state.nick(), "papo-wire");
    assert!(matches!(
        events.next().await,
        Some((Event::Disconnected(_), _))
    ));
}

#[tokio::test(start_paused = true)]
async fn a_cut_off_ops_only_message_stays_restricted() {
    let (_client, mut events, mut wire) = wire(
        papo::Client::builder("irc://hostile.offline.test", "papo-wire")
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "message-tags",
        "batch",
        "draft/multiline=max-bytes=4096,max-lines=100",
    ])
    .await;
    while !matches!(events.next().await, Some((Event::Connected(_), _))) {}
    wire.server(":hostile.offline.test 005 papo-wire STATUSMSG=~&@%+ :are supported")
        .await;

    for line in [
        "@msgid=RKi9Cx;time=2026-10-15T12:00:01.000Z :oppy!~u@cloak BATCH +keys draft/multiline @#ops",
        "@batch=keys :oppy!~u@cloak PRIVMSG @#ops :deploy key rotated",
        "@batch=keys :oppy!~u@cloak PRIVMSG @#ops :new key is in the vault",
    ] {
        wire.server(line).await;
    }
    wire.close().await;

    let event = loop {
        match events.next().await.map(|(event, _)| event) {
            Some(Event::Other(_)) => {}
            Some(event) => break event,
            None => panic!("driver stopped before the partial message"),
        }
    };
    let Event::Restricted(restricted) = event else {
        panic!("expected a restricted partial message, got {event:?}")
    };
    assert_eq!(restricted.audience.symbol(), '@');
    let Some(Incomplete::Message(message)) = restricted.line.as_incomplete() else {
        panic!("expected an incomplete message, got {restricted:?}")
    };
    assert_eq!(message.text, "deploy key rotated\nnew key is in the vault");
    assert_eq!(message.conversation(), "#ops");
}
