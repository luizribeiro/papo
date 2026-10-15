use std::time::Instant;

use papo_proto::MessageRef;

use crate::{Config, Handled, Outbox};

#[derive(Debug)]
pub(super) struct Registration {
    nick: String,
    deadline: Option<Instant>,
}

impl Registration {
    pub(super) fn new(config: &Config) -> Self {
        Self {
            nick: config.nick.clone(),
            deadline: None,
        }
    }

    pub(super) fn handle_connected(&mut self, now: Instant, config: &Config, outbox: &mut Outbox) {
        self.nick.clone_from(&config.nick);
        self.deadline = Some(now + config.registration_timeout);
        outbox.transmit_raw(b"CAP LS 302\r\n".to_vec());
        if let Some(password) = &config.password {
            outbox.transmit("PASS", [password]);
        }
        outbox.transmit("NICK", [&config.nick]);
        outbox.transmit("USER", [&config.username, "0", "*", &config.realname]);
    }

    pub(super) fn handle_message(&mut self, message: &MessageRef<'_>) -> Handled {
        if !message.command_is("001") {
            return Handled::No;
        }
        if let Some(nick) = message.param(0) {
            nick.clone_into(&mut self.nick);
        }
        self.deadline = None;
        Handled::Yes
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
}
