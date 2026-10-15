use std::time::Instant;

use papo_proto::MessageRef;

use crate::{Config, Handled, Outbox};

#[derive(Debug)]
pub(super) struct Registration {
    nick: String,
    deadline: Option<Instant>,
    next_alt_nick: usize,
}

impl Registration {
    pub(super) fn new(config: &Config) -> Self {
        Self {
            nick: config.nick.clone(),
            deadline: None,
            next_alt_nick: 0,
        }
    }

    pub(super) fn handle_connected(&mut self, now: Instant, config: &Config, outbox: &mut Outbox) {
        self.nick.clone_from(&config.nick);
        self.deadline = Some(now + config.registration_timeout);
        self.next_alt_nick = 0;
        outbox.transmit_raw(b"CAP LS 302\r\n".to_vec());
        if let Some(password) = &config.password {
            outbox.transmit("PASS", [password]);
        }
        outbox.transmit("NICK", [&config.nick]);
        outbox.transmit("USER", [&config.username, "0", "*", &config.realname]);
    }

    pub(super) fn handle_message(
        &mut self,
        message: &MessageRef<'_>,
        config: &Config,
        nicklen: Option<usize>,
        outbox: &mut Outbox,
    ) -> Handled {
        if message.command_is("001") {
            if let Some(nick) = message.param(0) {
                nick.clone_into(&mut self.nick);
            }
            self.deadline = None;
            Handled::Yes
        } else if self.deadline.is_some()
            && ["432", "433", "436", "437"]
                .iter()
                .any(|numeric| message.command_is(numeric))
        {
            if let Some(nick) = config.alt_nicks.get(self.next_alt_nick) {
                nick.clone_into(&mut self.nick);
                self.next_alt_nick += 1;
            } else {
                self.nick = append_underscore(&self.nick, nicklen.unwrap_or(30));
            }
            outbox.transmit("NICK", [&self.nick]);
            Handled::Yes
        } else {
            Handled::No
        }
    }

    pub(super) fn handle_timeout(&mut self, now: Instant, outbox: &mut Outbox) {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.deadline = None;
            outbox.close("IRC registration timed out before 001".to_owned());
        }
    }

    pub(super) const fn next_deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub(super) fn handle_disconnected(&mut self) {
        self.deadline = None;
    }

    pub(super) fn nick(&self) -> &str {
        &self.nick
    }

    #[allow(clippy::unused_self)]
    pub(super) const fn authentication_finished(&self) -> bool {
        true
    }
}

fn append_underscore(nick: &str, limit: usize) -> String {
    let underscores = nick.bytes().rev().take_while(|byte| *byte == b'_').count() + 1;
    let suffix_len = underscores.min(limit);
    let mut stem = nick.trim_end_matches('_').to_owned();
    let stem_limit = limit - suffix_len;
    while stem.len() > stem_limit {
        stem.pop();
    }
    stem.push_str(&"_".repeat(suffix_len));
    stem
}
