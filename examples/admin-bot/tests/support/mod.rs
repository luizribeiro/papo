#![allow(dead_code)]

use std::{
    io::{BufRead as _, BufReader},
    process::Stdio,
    sync::mpsc,
    time::Duration,
};

use papo::{Client, Endpoint, Event, Events, Message, ModeAction, ModeArg, Sasl};
use papo_test_server::ServerInfo;

pub(super) const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

pub(super) struct AdminBot {
    child: std::process::Child,
    errors: mpsc::Receiver<String>,
}

impl AdminBot {
    pub(super) fn start(server: &ServerInfo, nick: &str, admin: &str, channel: &str) -> Self {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_admin-bot"))
            .arg(format!("irc://{}", server.plaintext_addr()))
            .env("IRC_NICK", nick)
            .env("IRC_ADMIN", admin)
            .env("IRC_CHANNEL", channel)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = child.stderr.take().unwrap();
        let (send, errors) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Self { child, errors }
    }

    pub(super) fn next_error(&self) -> String {
        self.errors
            .recv_timeout(Duration::from_secs(5))
            .expect("admin bot did not log an action error")
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
    let mut builder = Client::builder(endpoint, nick).autojoin(channels);
    if authenticated {
        builder = builder
            .allow_insecure_auth(true)
            .sasl(Sasl::plain(nick, PASSWORD));
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
    wait_for(events, text, |event| match event {
        Event::Message(message) if message.text == text => Some(message),
        _ => None,
    })
    .await
}

pub(super) async fn wait_message_starting_with(events: &mut Events, text: &str) -> Message {
    wait_for(events, text, |event| match event {
        Event::Message(message) if message.text.starts_with(text) => Some(message),
        _ => None,
    })
    .await
}

pub(super) async fn wait_op(events: &mut Events, channel: &str, nick: &str) {
    wait_for(events, format!("+o {nick}"), |event| match event {
        Event::ModeChanged(changed)
            if matches!(&changed.subject, papo::Subject::Channel(name) if name == channel)
                && changed.changes.iter().any(|change| {
                    change.action == ModeAction::Add
                        && change.mode == 'o'
                        && matches!(&change.arg, Some(ModeArg::Member { nick: value, .. }) if value == nick)
                }) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
}

pub(super) async fn wait_join(events: &mut Events, nick: &str) {
    wait_for(events, format!("{nick} to join"), |event| {
        matches!(event, Event::Joined(ref joined) if joined.actor.nick() == nick).then_some(())
    })
    .await;
}

async fn wait_for<T>(
    events: &mut Events,
    expected: impl std::fmt::Display,
    mut match_event: impl FnMut(Event) -> Option<T>,
) -> T {
    loop {
        match events.next().await {
            Some(Event::Disconnected(disconnected)) => {
                panic!("waiting for {expected} failed: {disconnected}")
            }
            Some(event) => {
                if let Some(value) = match_event(event) {
                    return value;
                }
            }
            None => panic!("driver stopped while waiting for {expected}"),
        }
    }
}
