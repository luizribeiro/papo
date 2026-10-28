//! Highest-layer retained typing return-address regressions.

use papo::{Event, Reconnect};
use papo_testing::wire;

async fn next_active_event(events: &mut papo::Events) -> Event {
    loop {
        let event = events.next().await.expect("client stopped");
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
        let state = client.state();
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
        let state = client.state();
        let conversation = state.name_rules().unwrap().conversation("#ops").unwrap();
        state.typing(conversation).next().unwrap().return_address()
    };
    client.reply(operators, "ops pong").await.unwrap();
    assert_eq!(wire.next_sent().await, "PRIVMSG @#ops :ops pong");
}
