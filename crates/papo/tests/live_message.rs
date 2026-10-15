//! Message replies against the runner-managed Ergo server.
#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use std::time::Duration;

use papo::{Client, Event, MessageEvent};
use papo_proto::Source;
use papo_test_server::{from_env, unique};
use tokio::time::timeout;

#[tokio::test(flavor = "current_thread")]
async fn reply_between_two_clients_carries_the_original_msgid() {
    timeout(Duration::from_secs(10), async {
        let server = from_env();
        let address = format!("irc://{}", server.plaintext_addr());
        let channel = format!("#{}", unique("papo-reply"));
        let nick_a = unique("papo-sender");
        let nick_b = unique("papo-replier");

        let (client_a, mut events_a) = Client::builder(&address)
            .nick(&nick_a)
            .channels([&channel])
            .connect()
            .await
            .unwrap();
        wait_ready(&mut events_a, &nick_a, &channel).await;

        let (client_b, mut events_b) = Client::builder(&address)
            .nick(&nick_b)
            .channels([&channel])
            .connect()
            .await
            .unwrap();
        wait_ready(&mut events_b, &nick_b, &channel).await;

        client_a.privmsg(&channel, "!ping").await.unwrap();
        let incoming = wait_message(&mut events_b, |message| {
            message.text() == "!ping" && source_nick(message) == Some(nick_a.as_str())
        })
        .await;
        assert!(incoming.is_live());
        assert!(!incoming.is_private());
        let ping_msgid = incoming.msgid().expect("Ergo supplies msgid").to_owned();
        incoming.reply("pong").await.unwrap();

        let response = wait_message(&mut events_a, |message| {
            message.text() == "pong" && source_nick(message) == Some(nick_b.as_str())
        })
        .await;
        assert_eq!(response.reply_to(), Some(ping_msgid.as_str()));

        client_b
            .quit(Some("live reply test complete"))
            .await
            .unwrap();
        client_a
            .quit(Some("live reply test complete"))
            .await
            .unwrap();
    })
    .await
    .expect("live reply test did not finish within 10 seconds");
}

async fn wait_ready(events: &mut papo::Events, nick: &str, channel: &str) {
    let mut registered = false;
    loop {
        match events.next().await {
            Some(Event::Registered(welcome)) => registered = welcome.nick == nick,
            Some(Event::Joined {
                channel: joined,
                nick: joined_nick,
            }) if registered && joined == channel && joined_nick == nick => return,
            Some(Event::Disconnected { reason, .. }) => {
                panic!("disconnected while joining: {reason}")
            }
            Some(_) => {}
            None => panic!("driver stopped while joining"),
        }
    }
}

async fn wait_message(
    events: &mut papo::Events,
    accepted: impl Fn(&MessageEvent) -> bool,
) -> MessageEvent {
    loop {
        match events.next().await {
            Some(Event::Message(message)) if accepted(&message) => return message,
            Some(Event::Disconnected { reason, .. }) => {
                panic!("disconnected while waiting for a message: {reason}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for a message"),
        }
    }
}

fn source_nick(message: &MessageEvent) -> Option<&str> {
    match message.source() {
        Some(Source::User { nick, .. }) => Some(nick),
        Some(Source::Server(_)) | None => None,
    }
}
