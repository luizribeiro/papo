//! Client builder behavior against the runner-managed Ergo server.
#![cfg(all(sockets, any(tls, feature = "websocket")))]

mod support;

use support::within;

use papo::{Client, Event};
use papo_test_server::{from_env, unique};

#[cfg(tls)]
#[path = "live_builder/tls.rs"]
mod tls;

#[cfg(feature = "websocket")]
#[tokio::test(flavor = "current_thread")]
async fn connects_through_the_configured_websocket_url() {
    within(async {
        let server = from_env();
        let nick = unique("papo-builder-ws");
        let (client, mut events) = Client::builder(server.websocket_url(), support::nick(&nick))
            .connect()
            .await
            .unwrap();
        wait_connected(&mut events, &nick).await;
        client.quit().await.unwrap();
    })
    .await;
}

async fn wait_connected(events: &mut papo::Events, nick: &str) {
    loop {
        match events.next().await.map(|(event, _)| event) {
            Some(Event::Connected(welcome)) => {
                assert_eq!(welcome.nick, nick);
                return;
            }
            Some(Event::Disconnected(disconnected)) => {
                panic!("disconnected while waiting for registration: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for registration"),
        }
    }
}
