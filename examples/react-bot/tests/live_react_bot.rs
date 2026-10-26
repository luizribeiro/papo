//! Reaction bot behavior against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

use std::time::Duration;

use papo::{Client, Event};
use papo_test_server::{from_env, unique};
use tokio::time::timeout;

const CHANNEL: &str = "#papo";

#[tokio::test(flavor = "current_thread")]
async fn reacts_to_live_messages() {
    timeout(Duration::from_secs(20), async {
        let server = from_env();
        let bot_nick = unique("papo-react-bot");
        let server_url = format!("irc://{}", server.plaintext_addr());
        let (user, mut events) = Client::builder(server_url.clone(), unique("papo-react-user"))
            .autojoin([CHANNEL])
            .connect()
            .await
            .unwrap();
        wait_for(&mut events, |event| {
            matches!(event, Event::ChannelSynced(_))
        })
        .await;

        let _bot = Child(
            std::process::Command::new(env!("CARGO_BIN_EXE_react-bot"))
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

        let sent = user.privmsg(CHANNEL, "ship it").await.unwrap();
        let reaction = wait_for(&mut events, |event| matches!(event, Event::Reacted(_))).await;
        let Event::Reacted(reaction) = reaction else {
            unreachable!();
        };
        assert_eq!(&reaction.reacts_to, sent.msgid().unwrap());
        assert_eq!(reaction.reaction, "👍");

        user.quit("reaction bot test complete").await.unwrap();
    })
    .await
    .expect("live reaction bot test did not finish within 20 seconds");
}

struct Child(std::process::Child);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn wait_for(events: &mut papo::Events, mut wanted: impl FnMut(&Event) -> bool) -> Event {
    while let Some(event) = events.next().await {
        if wanted(&event) {
            return event;
        }
    }
    panic!("driver stopped before the expected event");
}
