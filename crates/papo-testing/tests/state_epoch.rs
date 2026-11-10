//! Offline wire coverage for event snapshots and connection epochs.

use std::{sync::Arc, time::Duration};

use papo::{Event, HistoryReason, Reconnect, State};
use papo_testing::wire;

async fn next_connected(events: &mut papo::Events) -> (Event, Arc<State>) {
    loop {
        let item = events
            .next()
            .await
            .expect("driver stopped before connecting");
        if matches!(item.0, Event::Connected(_)) {
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
        if let Event::Disconnected(disconnected) = item.0 {
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
    let (first, first_state) = next_connected(&mut events).await;
    let Event::Connected(first_connected) = &first else {
        unreachable!()
    };
    assert!(!first_connected.reconnected);
    assert_eq!(first_connected.epoch, first_state.connection_epoch());
    assert!(client.is_current(&first_state));

    wire.close().await;
    let delay = next_disconnected(&mut events)
        .await
        .reconnect_in
        .expect("disconnect should retry");
    tokio::time::advance(delay).await;
    wire.register(&[]).await;
    let (second, second_state) = next_connected(&mut events).await;
    let Event::Connected(second_connected) = &second else {
        unreachable!()
    };
    assert!(second_connected.reconnected);
    assert_eq!(second_connected.epoch, second_state.connection_epoch());
    assert_ne!(
        first_state.connection_epoch(),
        second_state.connection_epoch()
    );
    assert!(!client.is_current(&first_state));
    assert!(client.is_current(&second_state));
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

    let (event, state) = events
        .next()
        .await
        .expect("history batch should produce an event");
    let Event::History(page) = &event else {
        panic!("expected history, got {event:?}")
    };
    assert_eq!(page.reason, HistoryReason::Replay);
    assert!(matches!(
        page.items.as_slice(),
        [Event::Joined(_), Event::NickChanged(_)]
    ));
    assert_eq!(state.nick(), "papo-wire");
    assert!(state.__channel_local("#archive").is_none());
    assert_eq!(client.snapshot().nick(), "papo-wire");
    assert!(client.snapshot().__channel_local("#archive").is_none());
}
