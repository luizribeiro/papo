#![allow(dead_code)]

use papo::{Client, Endpoint, Event, Events, Message, ModeAction, ModeArg, Sasl};
use papo_test_server::ServerInfo;

pub(super) const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

pub(super) struct AdminBot {
    child: std::process::Child,
}

impl AdminBot {
    pub(super) fn start(server: &ServerInfo, nick: &str, admin: &str, channel: &str) -> Self {
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_admin-bot"))
            .arg(format!("irc://{}", server.plaintext_addr()))
            .env("IRC_NICK", nick)
            .env("IRC_ADMIN", admin)
            .env("IRC_CHANNEL", channel)
            .spawn()
            .unwrap();
        Self { child }
    }
}

impl Drop for AdminBot {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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
    )
    .unwrap();
    let mut builder = Client::builder(endpoint).nick(nick).autojoin(channels);
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
            Some(Event::Message(message)) if message.text == text => return *message,
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
                            && matches!(change.arg.clone(), Some(ModeArg::Nick(value)) if value == nick)
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
