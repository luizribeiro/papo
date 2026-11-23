//! Offline wire regressions for server-controlled session collections.

use papo::Events;
use papo_testing::{Wire, wire};

async fn next_active_event(events: &mut Events) -> papo::Event {
    match events.next().await.map(|(event, _)| event) {
        Some(papo::Event::Disconnected(disconnected)) => {
            panic!("disconnected while waiting for an event: {disconnected}")
        }
        Some(event) => event,
        None => panic!("driver stopped while waiting for an event"),
    }
}

async fn connected(events: &mut Events) {
    while !matches!(next_active_event(events).await, papo::Event::Connected(_)) {}
}

/// Returns every event the client produced before a marker message from another user.
async fn events_before_marker(events: &mut Events, wire: &mut Wire) -> Vec<papo::Event> {
    wire.server(":dan!~dan@users.example PRIVMSG papo :end of probe")
        .await;
    let mut seen = Vec::new();
    loop {
        let event = next_active_event(events).await;
        if matches!(&event, papo::Event::Message(message) if message.text == "end of probe") {
            return seen;
        }
        seen.push(event);
    }
}

#[tokio::test]
async fn metadata_subscription_acknowledgements_are_bounded() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://hostile.offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&["draft/metadata-2=max-subs=50,max-keys=50"])
        .await;
    connected(&mut events).await;

    for index in 0..3_400 {
        wire.server(format!(
            ":hostile.offline.test 770 papo example.org/a{index} example.org/b{index} example.org/c{index}"
        ))
        .await;
    }
    events_before_marker(&mut events, &mut wire).await;

    let retained = client.snapshot().metadata_subscriptions().len();
    assert!(
        retained <= 10_000,
        "the server grew the subscription list to {retained} keys"
    );
}

#[tokio::test]
async fn unanswered_capability_request_is_sent_once_on_the_wire() {
    let (_client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&["multi-prefix"]).await;
    connected(&mut events).await;
    wire.server(":offline.test CAP papo NEW :away-notify").await;
    assert_eq!(wire.next_sent().await, "CAP REQ :away-notify");

    wire.server(":offline.test CAP papo NEW :away-notify").await;
    wire.server("PING :cap-update-complete").await;
    assert_eq!(wire.next_sent().await, "PONG cap-update-complete");

    wire.server(":offline.test CAP papo NAK :away-notify").await;
    wire.server(":offline.test CAP papo NEW :away-notify").await;
    assert_eq!(wire.next_sent().await, "CAP REQ :away-notify");
}
