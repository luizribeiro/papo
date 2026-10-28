use papo::Events;
use papo_testing::Wire;

pub async fn next_active_event(events: &mut Events) -> papo::Event {
    match events.next().await {
        Some(papo::Event::Disconnected(disconnected)) => {
            panic!("disconnected while waiting for an event: {disconnected}")
        }
        Some(event) => event,
        None => panic!("driver stopped while waiting for an event"),
    }
}

pub async fn connected(events: &mut Events) {
    while !matches!(next_active_event(events).await, papo::Event::Connected(_)) {}
}

#[allow(dead_code)]
pub async fn enable_monitor(events: &mut Events, wire: &mut Wire, limit: usize) {
    wire.server(format!(
        ":offline.test 005 papo MONITOR={limit} :are supported"
    ))
    .await;
    while !matches!(
        next_active_event(events).await,
        papo::Event::FeaturesChanged { .. }
    ) {}
}
