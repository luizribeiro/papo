//! A real client whose bouncer advertises MONITOR only after the connection finished.

mod support {
    pub mod direct;
}

use std::time::Duration;

use papo::Reconnect;
use papo_testing::wire;
use support::direct::{connected, next_active_event};

#[tokio::test]
async fn watch_reaches_the_server_when_monitor_is_advertised_late() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    let watch = client.query("alice").unwrap().watch();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), wire.next_sent())
            .await
            .is_err()
    );

    wire.server(":offline.test 005 papo BOUNCER_NETID=1 MONITOR=100 :are supported by this server")
        .await;
    assert_eq!(wire.next_sent().await, "MONITOR + alice");
    wire.server(":offline.test 730 papo :alice!~alice@cloak.example")
        .await;
    loop {
        if let papo::Event::MonitorOnline(online) = next_active_event(&mut events).await {
            assert_eq!(online.subject.nick(), "alice");
            break;
        }
    }
    assert!(watch.is_live());
}
