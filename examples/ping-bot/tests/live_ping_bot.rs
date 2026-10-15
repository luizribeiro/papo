//! Ping bot behavior against the runner-managed Ergo server.
#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use std::time::Duration;

use papo::{Client, Event, MessageEvent};
use papo_test_server::{ServerInfo, from_env, register_account, unique};
use tokio::time::{sleep, timeout};

const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

#[tokio::test(flavor = "current_thread")]
async fn answers_channel_and_private_pings() {
    timeout(Duration::from_secs(20), async {
        let server = from_env();
        let bot_nick = unique("papo-ping-bot");
        let user_nick = unique("papo-ping-user");
        let channel = format!("#{}", unique("papo-ping"));
        register_account(&server, &bot_nick, PASSWORD).unwrap();
        let bot = Bot::start(&server, &bot_nick, &channel).await;
        sleep(Duration::from_millis(500)).await;

        let address = format!("irc://{}", server.plaintext_addr());
        let (user, mut events) = Client::builder(&address)
            .nick(&user_nick)
            .channels([&channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut events, &channel).await;

        user.privmsg(&channel, "!ping").await.unwrap();
        let sent = wait_message(&mut events, "!ping").await;
        let message_id = sent.msgid().expect("Ergo supplies msgid").to_owned();
        let response = wait_message(&mut events, "pong").await;
        assert_eq!(response.reply_to(), Some(message_id.as_str()));

        user.privmsg(&bot_nick, "!ping").await.unwrap();
        let sent = wait_message(&mut events, "!ping").await;
        let message_id = sent.msgid().expect("Ergo supplies msgid").to_owned();
        let response = wait_message(&mut events, "pong").await;
        assert!(response.is_private());
        assert_eq!(response.reply_to(), Some(message_id.as_str()));

        user.quit(Some("ping bot test complete")).await.unwrap();
        drop(bot);
    })
    .await
    .expect("live ping bot test did not finish within 20 seconds");
}

struct Bot {
    #[cfg(not(target_os = "wasi"))]
    process: (std::process::Child, std::path::PathBuf),
    #[cfg(all(target_os = "wasi", not(target_env = "p1")))]
    _session: (Client, tokio::task::JoinHandle<()>),
}

impl Bot {
    #[cfg(not(target_os = "wasi"))]
    fn start(server: &ServerInfo, nick: &str, channel: &str) -> std::future::Ready<Self> {
        use std::process::Command;
        let password_file = std::env::temp_dir().join(format!("{nick}.password"));
        std::fs::write(&password_file, format!("{PASSWORD}\n")).unwrap();
        let address = format!("ircs://localhost:{}", server.tls_addr().port());
        let child = Command::new(env!("CARGO_BIN_EXE_ping-bot"))
            .env("IRC_SERVER", address)
            .env("IRC_NICK", nick)
            .env("IRC_PASSWORD_FILE", &password_file)
            .env("IRC_CA_FILE", server.ca_pem_path())
            .env("IRC_CHANNELS", channel)
            .spawn()
            .unwrap();
        std::future::ready(Self {
            process: (child, password_file),
        })
    }

    #[cfg(all(target_os = "wasi", not(target_env = "p1")))]
    async fn start(server: &ServerInfo, nick: &str, channel: &str) -> Self {
        let address = format!("ircs://localhost:{}", server.tls_addr().port());
        let (client, mut events) = Client::builder(&address)
            .tls_ca_file(server.ca_pem_path())
            .unwrap()
            .nick(nick)
            .sasl(papo::Sasl::plain(nick, PASSWORD))
            .require_sasl(true)
            .bot()
            .channels([channel])
            .connect()
            .await
            .unwrap();
        let task = tokio::spawn(async move {
            while let Some(event) = events.next().await {
                if let Event::Message(message) = event
                    && message.is_live()
                    && message.text() == "!ping"
                {
                    message.reply("pong").await.unwrap();
                }
            }
        });
        Self {
            _session: (client, task),
        }
    }
}

#[cfg(not(target_os = "wasi"))]
impl Drop for Bot {
    fn drop(&mut self) {
        let _ = self.process.0.kill();
        let _ = self.process.0.wait();
        let _ = std::fs::remove_file(&self.process.1);
    }
}

async fn wait_synced(events: &mut papo::Events, channel: &str) {
    loop {
        match events.next().await {
            Some(Event::ChannelSynced { channel: synced }) if synced == channel => return,
            Some(Event::Disconnected { reason, .. }) => panic!("join failed: {reason}"),
            Some(_) => {}
            None => panic!("driver stopped while joining"),
        }
    }
}

async fn wait_message(events: &mut papo::Events, text: &str) -> MessageEvent {
    loop {
        match events.next().await {
            Some(Event::Message(message)) if message.text() == text => return message,
            Some(Event::Disconnected { reason, .. }) => panic!("message failed: {reason}"),
            Some(_) => {}
            None => panic!("driver stopped while waiting for {text}"),
        }
    }
}
