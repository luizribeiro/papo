//! Live delivery of identity facts for a sender known through private messages.

use std::time::Duration;

use super::direct::{connected, next_active_event};
use papo::Reconnect;
use papo_testing::wire;

pub const FIRST: &str =
    "@msgid=ErgoDm1 :alice!~alice@cloak-7f3a.example PRIVMSG papo :is the release out?";

pub async fn start(caps: &[&str]) -> (papo::Client, papo::Events, papo_testing::Wire) {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(caps).await;
    connected(&mut events).await;
    (client, events, wire)
}

pub async fn message(
    events: &mut papo::Events,
    wire: &mut papo_testing::Wire,
    lines: &[&str],
) -> papo::event::Message {
    for line in lines {
        wire.server(*line).await;
    }
    loop {
        if let papo::Event::Message(message) = next_active_event(events).await {
            return message;
        }
    }
}

/// Feeds `lines` and waits until papo has read all of them.
#[allow(dead_code)]
pub async fn read(events: &mut papo::Events, wire: &mut papo_testing::Wire, lines: &[&str]) {
    for line in lines {
        wire.server(*line).await;
    }
    wire.server(":offline.test NOTICE papo :*** end of test burst")
        .await;
    loop {
        if matches!(next_active_event(events).await, papo::Event::ServerNotice(notice) if notice.text == "*** end of test burst")
        {
            return;
        }
    }
}

pub async fn assert_refused(
    client: papo::Client,
    wire: &mut papo_testing::Wire,
    message: papo::event::Message,
) {
    let reply = tokio::spawn(async move { client.reply(&message, "release approved").await });
    let written = tokio::time::timeout(Duration::from_millis(50), wire.next_sent()).await;
    assert!(
        written.is_err(),
        "the reply was written to the new holder: {written:?}"
    );
    assert!(matches!(reply.await.unwrap(), Err(papo::Error::TargetGone)));
}
