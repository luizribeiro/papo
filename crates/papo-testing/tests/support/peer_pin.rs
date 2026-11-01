use std::time::Duration;

use papo::{Event, Reconnect};

use super::direct::next_active_event;

pub fn assert_match(stored: &papo::PeerKey, current: &papo::PeerKey) {
    assert!(matches!(stored.check(current), papo::PeerKeyCheck::Match));
}

#[allow(dead_code)]
pub fn assert_mismatch(
    stored: &papo::PeerKey,
    current: &papo::PeerKey,
    reason: papo::PeerKeyMismatch,
) {
    assert!(matches!(
        stored.check(current),
        papo::PeerKeyCheck::Mismatch(actual) if actual == reason
    ));
}

#[allow(dead_code)]
pub fn reconnecting() -> Reconnect {
    Reconnect::exponential()
        .initial_delay(Duration::ZERO)
        .max_delay(Duration::ZERO)
        .max_attempts(2)
}

pub async fn join(events: &mut papo::Events, wire: &mut papo_testing::Wire, line: &str) {
    wire.server(line).await;
    assert!(matches!(next_active_event(events).await, Event::Joined(_)));
}
