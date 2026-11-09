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
