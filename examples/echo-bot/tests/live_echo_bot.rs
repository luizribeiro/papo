//! Echo bot behavior against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

use std::{path::PathBuf, time::Duration};

use papo::{Client, Endpoint, Event, Message, Typing};
use papo_test_server::{ServerInfo, from_env, register_account, unique};
use tokio::time::{sleep, timeout};

const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

#[tokio::test(flavor = "current_thread")]
async fn echoes_reacts_and_reports_typing_with_message_context() {
    timeout(Duration::from_secs(20), async {
        let server = from_env();
        let bot_nick = unique("papo-echo-bot");
        let user_nick = unique("papo-echo-user");
        let channel = format!("#{}", unique("papo-echo"));
        register_account(&server, &bot_nick, PASSWORD).unwrap();
        let _bot = Bot::start(&server, &bot_nick, &channel);
        sleep(Duration::from_millis(500)).await;

        let endpoint = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        );
        let (user, mut events) = Client::builder(endpoint)
            .nick(&user_nick)
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut events, &channel).await;

        user.privmsg(&channel, "!echo hello").await.unwrap();
        let command = wait_message(&mut events, "!echo hello").await;
        let command_id = command.msgid().expect("Ergo supplies msgid").to_owned();
        let response = wait_message(&mut events, "hello").await;
        assert_eq!(response.reply_to(), Some(command_id.as_str()));

        let paste = "first line\nsecond line\nthird line";
        user.privmsg(&channel, paste).await.unwrap();
        let original = wait_message(&mut events, paste).await;
        let paste_id = original.msgid().expect("Ergo supplies msgid").to_owned();
        let summary = format!("got 3 lines, {} bytes", paste.len());
        let counted = wait_message(&mut events, &summary).await;
        assert_eq!(counted.reply_to(), Some(paste_id.as_str()));
        wait_typing(&mut events, &channel, Typing::Active).await;
        let echoed = wait_message(&mut events, paste).await;
        assert_eq!(echoed.text(), paste);
        assert_eq!(echoed.reply_to(), Some(paste_id.as_str()));
        wait_typing(&mut events, &channel, Typing::Done).await;

        user.privmsg(&channel, "!react").await.unwrap();
        let command = wait_message(&mut events, "!react").await;
        let command_id = command.msgid().expect("Ergo supplies msgid").to_owned();
        let (emoji, reply_to) = wait_reaction(&mut events).await;
        assert_eq!(reply_to, command_id);
        assert_eq!(emoji.as_deref(), Some("👍"));

        user.quit(Some("echo bot test complete")).await.unwrap();
    })
    .await
    .expect("live echo bot test did not finish within 20 seconds");
}

struct Bot {
    child: std::process::Child,
    password: PathBuf,
}

impl Bot {
    fn start(server: &ServerInfo, nick: &str, channel: &str) -> Self {
        let password = std::env::temp_dir().join(format!("{nick}.password"));
        std::fs::write(&password, format!("{PASSWORD}\n")).unwrap();
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_echo-bot"))
            .env("IRC_SERVER", "localhost")
            .env("IRC_PORT", server.tls_addr().port().to_string())
            .env("IRC_NICK", nick)
            .env("IRC_PASSWORD_FILE", &password)
            .env("IRC_CA_FILE", server.ca_pem_path())
            .env("IRC_CHANNELS", channel)
            .spawn()
            .unwrap();
        Self { child, password }
    }
}

impl Drop for Bot {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.password);
    }
}

async fn wait_synced(events: &mut papo::Events, channel: &str) {
    loop {
        match events.next().await {
            Some(Event::ChannelSynced(papo::event::ChannelSynced {
                channel: joined, ..
            })) if joined == channel => return,
            Some(Event::Disconnected(disconnected)) => {
                panic!("join failed: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while joining"),
        }
    }
}

async fn wait_message(events: &mut papo::Events, text: &str) -> Message {
    loop {
        match events.next().await {
            Some(Event::Message(message)) if message.text() == text => return *message,
            Some(Event::Disconnected(disconnected)) => {
                panic!("message failed: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for {text}"),
        }
    }
}

async fn wait_typing(events: &mut papo::Events, target: &str, state: Typing) {
    loop {
        match events.next().await {
            Some(Event::TypingChanged(typing))
                if typing.target == target && typing.state == state =>
            {
                return;
            }
            Some(Event::Disconnected(disconnected)) => {
                panic!("typing failed: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for typing state"),
        }
    }
}

async fn wait_reaction(events: &mut papo::Events) -> (Option<String>, String) {
    loop {
        match events.next().await {
            Some(Event::Reacted(reaction)) => {
                return (
                    (!reaction.removed).then_some(reaction.reaction),
                    reaction.message.into_string(),
                );
            }
            Some(Event::Disconnected(disconnected)) => {
                panic!("reaction failed: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for a reaction"),
        }
    }
}
