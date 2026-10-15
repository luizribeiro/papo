//! Client builder behavior against the runner-managed Ergo server.
#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use std::time::Duration;

use papo::{Client, Event, Sasl};
use papo_proto::Message;
use papo_test_server::{from_env, register_account, unique};
use tokio::time::timeout;

#[tokio::test(flavor = "current_thread")]
async fn connects_with_sasl_and_applies_bot_defaults() {
    timeout(Duration::from_secs(10), async {
        let server = from_env();
        let account = unique("papo-builder");
        let channel = format!("#{}", unique("papo-builder"));
        let password = "q7Hx-2mZp_9vLcT4wRfB";
        register_account(&server, &account, password).unwrap();

        let (client, mut events) =
            Client::builder(&format!("ircs://localhost:{}", server.tls_addr().port()))
                .nick(&account)
                .sasl(Sasl::plain(&account, password))
                .require_sasl(true)
                .tls_ca_file(server.ca_pem_path())
                .unwrap()
                .bot()
                .channels([&channel])
                .connect()
                .await
                .unwrap();

        wait_registered(&mut events, &account).await;
        wait_joined(&mut events, &account, &channel).await;

        client
            .send(Message::new("WHOIS", [account.clone()]))
            .await
            .unwrap();
        wait_for(&mut events, |message| {
            message.command_is("335") && message.param(1) == Some(account.as_str())
        })
        .await;
        client.quit(Some("builder test complete")).await.unwrap();
    })
    .await
    .expect("builder live test did not finish within 10 seconds");
}

async fn wait_joined(events: &mut papo::Events, nick: &str, channel: &str) {
    loop {
        match events.next().await {
            Some(Event::Joined {
                channel: joined,
                nick: joined_nick,
            }) if joined == channel && joined_nick == nick => return,
            Some(Event::Disconnected { reason, .. }) => {
                panic!("disconnected while waiting for JOIN: {reason}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for JOIN"),
        }
    }
}

#[cfg(feature = "websocket")]
#[tokio::test(flavor = "current_thread")]
async fn connects_through_the_configured_websocket_url() {
    timeout(Duration::from_secs(10), async {
        let server = from_env();
        let nick = unique("papo-builder-ws");
        let (client, mut events) = Client::builder(server.websocket_url())
            .nick(&nick)
            .connect()
            .await
            .unwrap();
        wait_registered(&mut events, &nick).await;
        client
            .quit(Some("WebSocket builder test complete"))
            .await
            .unwrap();
    })
    .await
    .expect("WebSocket builder live test did not finish within 10 seconds");
}

async fn wait_registered(events: &mut papo::Events, nick: &str) {
    loop {
        match events.next().await {
            Some(Event::Registered(welcome)) => {
                assert_eq!(welcome.nick, nick);
                return;
            }
            Some(Event::Disconnected { reason, .. }) => {
                panic!("disconnected while waiting for registration: {reason}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for registration"),
        }
    }
}

async fn wait_for(
    events: &mut papo::Events,
    accepted: impl Fn(&papo_proto::MessageRef<'_>) -> bool,
) {
    loop {
        match events.next().await {
            Some(Event::Message(message)) if accepted(&message.as_ref()) => return,
            Some(Event::Disconnected { reason, .. }) => {
                panic!("disconnected while waiting: {reason}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for an IRC message"),
        }
    }
}
