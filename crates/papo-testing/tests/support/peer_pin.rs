use std::time::Duration;

use papo::{Event, Reconnect};

use super::direct::next_active_event;

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
