//! Client builder behavior against the runner-managed Ergo server.
#![cfg(sockets)]

mod support;

use support::within;

use papo::{Client, Endpoint, Event, Sasl};
use papo_test_server::{create_account, from_env, unique};

#[tokio::test(flavor = "current_thread")]
async fn connects_with_sasl_and_applies_bot_defaults() {
    within(async {
        let server = from_env();
        let account = unique("papo-builder");
        let channel = format!("#{}", unique("papo-builder"));
        let password = "q7Hx-2mZp_9vLcT4wRfB";
        create_account(&server, &account, password).unwrap();

        let (client, mut events) =
            Client::builder(Endpoint::tls("localhost", server.tls_addr().port()).unwrap())
                .nick(support::nick(&account))
                .sasl(Sasl::plain(&account, password))
                .tls_ca_pem(std::fs::read(server.ca_pem_path()).unwrap())
                .bot(true)
                .autojoin([&channel])
                .connect()
                .await
                .unwrap();

        wait_connected(&mut events, &account).await;
        wait_joined(&mut events, &account, &channel).await;

        assert!(client.whois(support::nick(&account)).await.unwrap().is_bot);
        client.quit("builder test complete").await.unwrap();
    })
    .await;
}

async fn wait_joined(events: &mut papo::Events, nick: &str, channel: &str) {
    loop {
        match events.next().await {
            Some(Event::Joined(papo::event::Joined {
                channel: joined,
                nick: joined_nick,
                ..
            })) if joined == channel && joined_nick == nick => return,
            Some(Event::Disconnected(disconnected)) => {
                panic!("disconnected while waiting for JOIN: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for JOIN"),
        }
    }
}

#[cfg(feature = "websocket")]
#[tokio::test(flavor = "current_thread")]
async fn connects_through_the_configured_websocket_url() {
    within(async {
        let server = from_env();
        let nick = unique("papo-builder-ws");
        let (client, mut events) =
            Client::builder(Endpoint::websocket(server.websocket_url()).unwrap())
                .nick(support::nick(&nick))
                .connect()
                .await
                .unwrap();
        wait_connected(&mut events, &nick).await;
        client
            .quit("WebSocket builder test complete")
            .await
            .unwrap();
    })
    .await;
}

async fn wait_connected(events: &mut papo::Events, nick: &str) {
    loop {
        match events.next().await {
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
