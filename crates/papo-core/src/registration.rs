use web_time::Instant;

use papo_proto::{Isupport, Message, MessageRef, escape_tag_value};

use crate::{Capabilities, CloseKind, Config, Event, Handled, Outbox, Welcome};

const WELCOME_BURST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Debug)]
pub(super) struct Registration {
    nick: String,
    deadline: Option<Instant>,
    welcome_deadline: Option<Instant>,
    welcome_text: Option<String>,
    next_alt_nick: usize,
    registered: bool,
    webirc_pending: bool,
}

impl Registration {
    pub(super) fn new(config: &Config) -> Self {
        Self {
            nick: config.nick.clone(),
            deadline: None,
            welcome_deadline: None,
            welcome_text: None,
            next_alt_nick: 0,
            registered: false,
            webirc_pending: false,
        }
    }

    pub(super) fn handle_connected(&mut self, now: Instant, config: &Config, outbox: &mut Outbox) {
        self.nick.clone_from(&config.nick);
        self.deadline = Some(now + config.registration_timeout);
        self.welcome_deadline = None;
        self.welcome_text = None;
        self.next_alt_nick = 0;
        self.registered = false;
        self.webirc_pending = config.webirc.is_some();
        if let Some(webirc) = &config.webirc {
            let Some(password) = webirc.password.expose() else {
                outbox.close(
                    CloseKind::Other,
                    "WEBIRC password contains bytes rather than text".to_owned(),
                );
                return;
            };
            let ip = if webirc.ip.starts_with(':') {
                format!("0{}", webirc.ip)
            } else {
                webirc.ip.clone()
            };
            let hostname = webirc.hostname.as_deref().unwrap_or(&ip);
            let mut params = vec![password, webirc.gateway.as_str(), hostname, ip.as_str()];
            let options = webirc
                .options
                .iter()
                .map(|(name, value)| match value {
                    Some(value) => format!("{name}={}", escape_tag_value(value)),
                    None => name.clone(),
                })
                .collect::<Vec<_>>()
                .join(" ");
            if !options.is_empty() {
                params.push(&options);
            }
            let Ok(line) = Message::new("WEBIRC", params).to_line() else {
                outbox.close(CloseKind::Other, "invalid WEBIRC configuration".to_owned());
                return;
            };
            outbox.transmit_raw(line.into());
        }
        outbox.transmit_raw(b"CAP LS 302\r\n".to_vec());
        if let Some(password) = &config.password {
            let Some(password) = password.expose() else {
                outbox.close(
                    CloseKind::Other,
                    "server password contains bytes rather than text".to_owned(),
                );
                return;
            };
            outbox.transmit("PASS", [password]);
        }
        outbox.transmit("NICK", [&config.nick]);
        outbox.transmit("USER", [&config.username, "0", "*", &config.realname]);
    }

    pub(super) fn handle_message(
        &mut self,
        now: Instant,
        message: &MessageRef<'_>,
        config: &Config,
        caps: &Capabilities,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> Handled {
        if self.webirc_pending {
            self.webirc_pending = false;
            if message.command_is("ERROR") {
                outbox.close(
                    CloseKind::WebIrcRejected,
                    message
                        .param(0)
                        .unwrap_or("server rejected WEBIRC")
                        .to_owned(),
                );
                return Handled::Yes;
            }
        }
        if message.command_is("001") {
            if let Some(nick) = message.param(0) {
                nick.clone_into(&mut self.nick);
            }
            self.welcome_text = Some(message.param(1).unwrap_or_default().to_owned());
            self.deadline = None;
            self.welcome_deadline = Some(now + WELCOME_BURST_TIMEOUT);
            Handled::No
        } else if self.welcome_deadline.is_some()
            && (message.command_is("376") || message.command_is("422"))
        {
            self.complete(caps, isupport, outbox);
            Handled::Yes
        } else if self.deadline.is_some()
            && (message.command_is("464") || message.command_is("465"))
        {
            outbox.close(
                if message.command_is("464") {
                    CloseKind::PasswordIncorrect
                } else {
                    CloseKind::Banned
                },
                message.param(1).unwrap_or_default().to_owned(),
            );
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
                self.nick = append_underscore(&self.nick, isupport.nicklen().unwrap_or(30));
            }
            outbox.transmit("NICK", [&self.nick]);
            Handled::Yes
        } else {
            Handled::No
        }
    }

    pub(super) fn handle_timeout(
        &mut self,
        now: Instant,
        caps: &Capabilities,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.deadline = None;
            outbox.close(
                CloseKind::RegistrationTimeout,
                "IRC registration timed out before 001".to_owned(),
            );
        } else if self
            .welcome_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.complete(caps, isupport, outbox);
        }
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        [self.deadline, self.welcome_deadline]
            .into_iter()
            .flatten()
            .min()
    }

    pub(super) fn handle_disconnected(&mut self) {
        self.deadline = None;
        self.welcome_deadline = None;
        self.welcome_text = None;
        self.registered = false;
        self.webirc_pending = false;
    }

    pub(super) fn nick(&self) -> &str {
        &self.nick
    }

    pub(super) const fn is_registered(&self) -> bool {
        self.registered
    }

    pub(super) const fn is_in_welcome_burst(&self) -> bool {
        self.welcome_deadline.is_some()
    }

    pub(super) fn update_nick(&mut self, nick: &str) {
        nick.clone_into(&mut self.nick);
    }

    fn complete(&mut self, caps: &Capabilities, isupport: &Isupport, outbox: &mut Outbox) {
        self.welcome_deadline = None;
        self.registered = true;
        outbox.event(Event::Registered(Welcome {
            nick: self.nick.clone(),
            text: self.welcome_text.take().unwrap_or_default(),
            caps: caps.clone(),
            isupport: isupport.clone(),
        }));
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
