//! Ping bot behavior against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

use std::time::Duration;

use papo::{Client, Event, Message};
use papo_test_server::{from_env, unique};
use tokio::time::timeout;

const CHANNEL: &str = "#papo";

#[tokio::test(flavor = "current_thread")]
async fn joins_and_replies_to_channel_pings() {
    timeout(Duration::from_secs(20), async {
        let server = from_env();
        let bot_nick = unique("papo-ping-bot");
        let user_nick = unique("papo-ping-user");
        let server_url = format!("irc://{}", server.plaintext_addr());
        let (user, mut events) = Client::builder(server_url.clone(), user_nick)
            .autojoin([CHANNEL])
            .connect()
            .await
            .unwrap();
        wait_for(&mut events, |event| {
            matches!(event, Event::ChannelSynced(_))
        })
        .await;

        let _bot = Child(
            std::process::Command::new(env!("CARGO_BIN_EXE_ping-bot"))
                .arg(server_url)
                .env("IRC_NICK", &bot_nick)
                .spawn()
                .unwrap(),
        );
        wait_for(
            &mut events,
            |event| matches!(event, Event::Joined(joined) if joined.actor.nick() == bot_nick.as_str()),
        )
        .await;

        let sent = user.privmsg(CHANNEL, "!ping").await.unwrap();
        let reply = wait_message(&mut events, "pong").await;
        assert_eq!(reply.in_reply_to.as_ref(), sent.msgid());

        user.quit("ping bot test complete").await.unwrap();
    })
    .await
    .expect("live ping bot test did not finish within 20 seconds");
}

struct Child(std::process::Child);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn wait_for(events: &mut papo::Events, mut wanted: impl FnMut(&Event) -> bool) {
    while let Some(event) = events.next().await {
        if wanted(&event) {
            return;
        }
    }
    panic!("driver stopped before the expected event");
}

async fn wait_message(events: &mut papo::Events, text: &str) -> Message {
    loop {
        match events.next().await {
            Some(Event::Message(message)) if message.text == text => return message,
            Some(_) => {}
            None => panic!("driver stopped while waiting for {text}"),
        }
    }
}
