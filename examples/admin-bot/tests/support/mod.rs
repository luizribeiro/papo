#![allow(dead_code)]

use std::{path::PathBuf, time::Duration};

use papo::{
    Client, Endpoint, Event, Events, LegacyModeAction as ModeAction, Message, ModeArg, Sasl,
};
use papo_test_server::ServerInfo;
use tokio::time::{sleep, timeout};

pub(super) const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

pub(super) struct AdminBot {
    child: std::process::Child,
    password: PathBuf,
}

impl AdminBot {
    pub(super) fn start(server: &ServerInfo, nick: &str, channels: &str, admins: &str) -> Self {
        let password = std::env::temp_dir().join(format!("{nick}.password"));
        std::fs::write(&password, format!("{PASSWORD}\n")).unwrap();
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_admin-bot"))
            .env("IRC_SERVER", "localhost")
            .env("IRC_PORT", server.tls_addr().port().to_string())
            .env("IRC_NICK", nick)
            .env("IRC_PASSWORD_FILE", &password)
            .env("IRC_CA_FILE", server.ca_pem_path())
            .env("IRC_CHANNELS", channels)
            .env("IRC_ADMINS", admins)
            .spawn()
            .unwrap();
        Self { child, password }
    }

    pub(super) async fn shutdown(&mut self) {
        let status = std::process::Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let status = timeout(Duration::from_secs(10), async {
            loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    return status;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("admin bot did not exit within 10 seconds");
        assert_eq!(status.code(), Some(0));
    }
}

impl Drop for AdminBot {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.password);
    }
}

pub(super) async fn connect(
    server: &ServerInfo,
    nick: &str,
    channels: &[String],
    authenticated: bool,
) -> (Client, Events) {
    let endpoint = Endpoint::plaintext(
        server.plaintext_addr().ip().to_string(),
        server.plaintext_addr().port(),
    );
    let mut builder = Client::builder(endpoint)
        .nick(papo::Casemapping::default().nick(nick))
        .autojoin(channels);
    if authenticated {
        builder = builder.sasl(Sasl::plain(nick, PASSWORD));
    }
    let (client, mut events) = builder.connect().await.unwrap();
    wait_synced(&mut events, channels).await;
    (client, events)
}

pub(super) async fn wait_synced(events: &mut Events, channels: &[String]) {
    let mut pending = channels.to_vec();
    while !pending.is_empty() {
        match events.next().await {
            Some(Event::ChannelSynced(papo::event::ChannelSynced { channel, .. })) => {
                if let Some(index) = pending.iter().position(|item| item == channel.as_ref()) {
                    pending.swap_remove(index);
                }
            }
            Some(Event::Disconnected(disconnected)) => {
                panic!("join failed: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while joining"),
        }
    }
}

pub(super) async fn wait_message(events: &mut Events, text: &str) -> Message {
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

pub(super) async fn wait_op(events: &mut Events, channel: &str, nick: &str) {
    loop {
        match events.next().await {
            Some(Event::ModeChanged(changed))
                if matches!(&changed.subject, papo::Subject::Channel(name) if name == channel)
                    && changed.changes.iter().any(|change| {
                        change.action == ModeAction::Add
                            && change.mode == 'o'
                            && matches!(&change.typed_arg, Some(ModeArg::Nick(value)) if value == nick)
                    }) =>
            {
                return;
            }
            Some(Event::Disconnected(disconnected)) => {
                panic!("mode failed: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for +o {nick}"),
        }
    }
}
