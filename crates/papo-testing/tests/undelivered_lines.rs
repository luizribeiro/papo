//! Offline wire coverage for unsolicited presence and metadata.

use papo::{Events, Reconnect};
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
async fn away_reply_for_an_unseen_user_is_delivered() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["message-tags", "server-time"]).await;
    connected(&mut events).await;

    client.privmsg("carol", "are you around?").await.unwrap();
    assert_eq!(wire.next_sent().await, "PRIVMSG carol :are you around?");
    wire.server(":offline.test 301 papo carol :Gone fishing until Monday")
        .await;

    let seen = events_before_marker(&mut events, &mut wire).await;
    assert!(
        seen.iter()
            .any(|event| matches!(event, papo::Event::AwayChanged(away)
            if away.actor.nick().as_str() == "carol"
                && matches!(&away.away, papo::AwayStatus::Away { message: Some(text), .. }
                    if text == "Gone fishing until Monday"))),
        "RPL_AWAY for carol became no event: {seen:?}"
    );
}

#[tokio::test]
async fn presence_lines_for_untracked_users_are_delivered() {
    let (_client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "account-notify",
        "away-notify",
        "extended-join",
        "no-implicit-names",
    ])
    .await;
    connected(&mut events).await;
    wire.server(":papo!~papo@localhost JOIN #papo * :papo")
        .await;
    assert_eq!(wire.next_sent().await, "NAMES #papo");
    events_before_marker(&mut events, &mut wire).await;

    for (line, marker) in [
        (
            ":alice!~alice@users.example AWAY :Gone fishing until Monday",
            "Gone fishing until Monday",
        ),
        (
            ":alice!~alice@users.example ACCOUNT alice-services",
            "alice-services",
        ),
    ] {
        wire.server(line).await;
        let seen = events_before_marker(&mut events, &mut wire).await;
        assert!(
            format!("{seen:?}").contains(marker),
            "{line} became no event: {seen:?}"
        );
    }
}

#[tokio::test]
async fn postponed_metadata_sync_is_delivered() {
    let (_client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never())
            .metadata_subscriptions([papo::MetadataKey::new("avatar")]),
    );
    wire.register(&["draft/metadata-2"]).await;
    connected(&mut events).await;
    wire.server(":papo!~papo@localhost JOIN #papo").await;
    wire.server(":offline.test 353 papo = #papo :@papo alice")
        .await;
    wire.server(":offline.test 366 papo #papo :End of /NAMES list.")
        .await;
    events_before_marker(&mut events, &mut wire).await;

    wire.server(":offline.test 774 papo #papo 30").await;

    let seen = events_before_marker(&mut events, &mut wire).await;
    assert!(
        seen.iter()
            .any(|event| matches!(event, papo::Event::Other(line)
            if line.numeric() == Some(774)
                && matches!(line.source(), Some(papo::LineSource::Server("offline.test")))
                && line.get(1) == Some("#papo")
                && line.get(2) == Some("30"))),
        "RPL_METADATASYNCLATER for #papo became no event"
    );
}
