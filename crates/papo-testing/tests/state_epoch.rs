//! Offline wire coverage for event snapshots and connection epochs.

use std::time::Duration;

use papo::{Event, EventWithState, HistoryReason, Reconnect};
use papo_testing::wire;

async fn next_connected(events: &mut papo::Events) -> EventWithState {
    loop {
        let item = events
            .next()
            .await
            .expect("driver stopped before connecting");
        if matches!(item.event, Event::Connected(_)) {
            return item;
        }
    }
}

async fn next_disconnected(events: &mut papo::Events) -> papo::Disconnected {
    loop {
        let item = events
            .next()
            .await
            .expect("driver stopped before reporting the disconnection");
        if let Event::Disconnected(disconnected) = item.event {
            return disconnected;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn reconnect_replaces_the_current_connection_epoch() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(
                Reconnect::exponential()
                    .max_delay(Duration::from_secs(1))
                    .max_attempts(2),
            ),
    );
    wire.register(&[]).await;
    let first = next_connected(&mut events).await;
    let Event::Connected(first_connected) = &first.event else {
        unreachable!()
    };
    assert!(!first_connected.reconnected);
    assert_eq!(first_connected.epoch, first.state.connection_epoch());
    assert!(client.is_current(&first.state));

    wire.close().await;
    let delay = next_disconnected(&mut events)
        .await
        .reconnect_in
        .expect("disconnect should retry");
    tokio::time::advance(delay).await;
    wire.register(&[]).await;
    let second = next_connected(&mut events).await;
    let Event::Connected(second_connected) = &second.event else {
        unreachable!()
    };
    assert!(second_connected.reconnected);
    assert_eq!(second_connected.epoch, second.state.connection_epoch());
    assert_ne!(
        first.state.connection_epoch(),
        second.state.connection_epoch()
    );
    assert!(!client.is_current(&first.state));
    assert!(client.is_current(&second.state));
}

#[tokio::test(start_paused = true)]
async fn replayed_history_is_marked_without_changing_live_state() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["message-tags", "batch", "server-time", "draft/chathistory"])
        .await;
    _ = next_connected(&mut events).await;

    for line in [
        ":offline.test BATCH +history-42 chathistory #archive",
        "@batch=history-42;time=2026-10-15T12:00:01.000Z :papo-wire!papo@localhost JOIN #archive * :Papo Wire",
        "@batch=history-42;time=2026-10-15T12:00:02.000Z :papo-wire!papo@localhost NICK :archived-papo",
        ":offline.test BATCH -history-42",
    ] {
        wire.server(line).await;
    }

    let item = events
        .next()
        .await
        .expect("history batch should produce an event");
    let Event::History(page) = &item.event else {
        panic!("expected history, got {:?}", item.event)
    };
    assert_eq!(page.reason, HistoryReason::Replay);
    assert!(matches!(
        page.items.as_slice(),
        [Event::Joined(_), Event::NickChanged(_)]
    ));
    assert_eq!(item.state.nick(), "papo-wire");
    assert!(item.state.channel("#archive").is_none());
    assert_eq!(client.snapshot().nick(), "papo-wire");
    assert!(client.snapshot().channel("#archive").is_none());
}
