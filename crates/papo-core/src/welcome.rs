use web_time::Instant;

use papo_proto::{
    Advertised, BuildError, Isupport, Line, LineRef, StandardReply, StandardReplyKind,
};

use crate::{
    Capabilities, CloseKind, Config, Connection, Event, Handled, Outbox, Welcome, cap, webirc_line,
};

const WELCOME_BURST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_MOTD_LINES: usize = 1_000;
const ERGO_WEBIRC_REJECTION: &str =
    "WEBIRC command is not usable from your address or incorrect password given";

#[derive(Debug)]
pub(super) struct WelcomeState {
    origin: papo_proto::Origin,
    nick: String,
    deadline: Option<Instant>,
    welcome_deadline: Option<Instant>,
    welcome_text: Option<String>,
    motd_lines: Option<Vec<String>>,
    next_alt_nick: usize,
    connected: bool,
    webirc_pending: bool,
    password_sent: bool,
    registration_error: Option<BuildError>,
}

impl WelcomeState {
    pub(super) fn new(config: &Config) -> Self {
        Self {
            origin: config.origin,
            nick: config.nick.clone(),
            deadline: None,
            welcome_deadline: None,
            welcome_text: None,
            motd_lines: None,
            next_alt_nick: 0,
            connected: false,
            webirc_pending: false,
            password_sent: false,
            registration_error: registration_error(config),
        }
    }

    pub(super) fn handle_connected(&mut self, now: Instant, config: &Config, outbox: &mut Outbox) {
        if let Some(error) = self.registration_error {
            outbox.close(
                CloseKind::Other,
                format!("invalid registration configuration: {error}"),
            );
            return;
        }
        self.nick.clone_from(&config.nick);
        self.deadline = crate::deadline(now, config.welcome_timeout);
        self.welcome_deadline = None;
        self.welcome_text = None;
        self.motd_lines = None;
        self.next_alt_nick = 0;
        self.connected = false;
        self.webirc_pending = config.webirc.is_some();
        self.password_sent = false;
        if let Some(webirc) = &config.webirc {
            let Ok(line) = webirc_line(webirc) else {
                outbox.close(CloseKind::Other, "invalid WEBIRC configuration".to_owned());
                return;
            };
            outbox.transmit_raw(line);
        }
        if let Ok(line) = Line::new("CAP").param("LS").param("302").encode() {
            outbox.transmit_raw(line);
        }
        outbox.transmit("NICK", [&config.nick]);
        outbox.transmit("USER", [&config.username, "0", "*", &config.realname]);
    }

    pub(super) fn handle_message(
        &mut self,
        now: Instant,
        message: &LineRef<'_>,
        config: &Config,
        caps: &Capabilities,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> Handled {
        if self.webirc_pending
            && message.command_is("ERROR")
            && message.param(0) == Some(ERGO_WEBIRC_REJECTION)
        {
            outbox.close(CloseKind::WebIrcRejected, ERGO_WEBIRC_REJECTION.to_owned());
            return Handled::Yes;
        }
        if config.selected_bouncer_network().is_some()
            && StandardReply::parse(message).is_some_and(|reply| {
                reply.kind == StandardReplyKind::Fail
                    && reply.command.eq_ignore_ascii_case("BOUNCER")
                    && reply.code.eq_ignore_ascii_case("INVALID_NETID")
            })
        {
            outbox.close(
                CloseKind::BouncerBindingFailed,
                "the bouncer rejected the selected network ID".to_owned(),
            );
            return Handled::Yes;
        }
        if message.command_is("465") {
            outbox.close(
                CloseKind::Banned,
                message.param(1).unwrap_or_default().to_owned(),
            );
            Handled::Yes
        } else if message.command_is("001") {
            self.webirc_pending = false;
            if let Some(nick) = message.param(0) {
                nick.clone_into(&mut self.nick);
            }
            self.welcome_text = Some(message.param(1).unwrap_or_default().to_owned());
            self.motd_lines = None;
            self.deadline = None;
            self.welcome_deadline = crate::deadline(now, WELCOME_BURST_TIMEOUT);
            Handled::No
        } else if self.welcome_deadline.is_some() && message.command_is("375") {
            self.motd_lines = Some(Vec::new());
            Handled::Yes
        } else if self.welcome_deadline.is_some() && message.command_is("372") {
            let Some(lines) = self.motd_lines.as_mut() else {
                return Handled::No;
            };
            if lines.len() < MAX_MOTD_LINES {
                lines.push(message.param(1).unwrap_or_default().to_owned());
            }
            Handled::Yes
        } else if self.welcome_deadline.is_some()
            && (message.command_is("376") || message.command_is("422"))
        {
            if message.command_is("376")
                && let Some(lines) = self.motd_lines.take()
            {
                outbox.event(Event::Motd { lines });
            }
            self.complete(config, caps, isupport, outbox);
            Handled::Yes
        } else if self.deadline.is_some() && message.command_is("464") {
            outbox.close(
                CloseKind::PasswordIncorrect,
                message.param(1).unwrap_or_default().to_owned(),
            );
            Handled::Yes
        } else if self.deadline.is_some() && message.command_is("463") {
            outbox.close(
                CloseKind::ConnectionRestricted,
                message.param(1).unwrap_or_default().to_owned(),
            );
            Handled::Yes
        } else if self.deadline.is_some() && rejects_nick(message) {
            if let Some(nick) = config.alt_nicks.get(self.next_alt_nick) {
                nick.clone_into(&mut self.nick);
                self.next_alt_nick += 1;
                outbox.transmit("NICK", [&self.nick]);
            } else {
                let nick = match isupport.name_limit(papo_proto::NameLimit::Nick) {
                    papo_proto::KnownLimit::Known(limit) => append_underscore(&self.nick, limit),
                    papo_proto::KnownLimit::Unknown => format!("{}_", self.nick),
                };
                if nick == self.nick {
                    outbox.close(
                        CloseKind::NickCandidatesExhausted,
                        "nick candidates exhausted".to_owned(),
                    );
                } else {
                    self.nick = nick;
                    outbox.transmit("NICK", [&self.nick]);
                }
            }
            Handled::Yes
        } else {
            Handled::No
        }
    }

    pub(super) fn handle_security_decision(
        message: &LineRef<'_>,
        config: &Config,
        connection: &Connection,
        outbox: &mut Outbox,
    ) -> Handled {
        if config.password.is_none()
            || crate::secure_credentials_allowed(config, connection)
            || !cap_is_unavailable(message)
        {
            return Handled::No;
        }
        outbox.close(
            CloseKind::CapabilityUnavailable,
            format!(
                "required IRC capability {} was unavailable",
                cap::STS.as_str()
            ),
        );
        Handled::Yes
    }

    pub(super) fn handle_cap_decision(
        &mut self,
        message: &LineRef<'_>,
        config: &Config,
        outbox: &mut Outbox,
    ) {
        if message.command_is("CAP")
            && message
                .param(1)
                .is_some_and(|command| command.eq_ignore_ascii_case("LS"))
            && message.param(2) != Some("*")
            && !outbox.close_requested
        {
            self.send_password(config, outbox);
        }
    }

    fn send_password(&mut self, config: &Config, outbox: &mut Outbox) {
        if self.password_sent {
            return;
        }
        self.password_sent = true;
        let Some(password) = &config.password else {
            return;
        };
        match Line::new("PASS").param(password).encode() {
            Ok(line) => outbox.transmit_raw(line),
            Err(error) => outbox.close(CloseKind::Other, format!("invalid PASS message: {error}")),
        }
    }

    pub(super) fn handle_timeout(
        &mut self,
        now: Instant,
        config: &Config,
        caps: &Capabilities,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.deadline = None;
            outbox.close(
                CloseKind::WelcomeTimeout,
                "IRC welcome timed out before 001".to_owned(),
            );
        } else if self
            .welcome_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.complete(config, caps, isupport, outbox);
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
        self.motd_lines = None;
        self.connected = false;
        self.webirc_pending = false;
        self.password_sent = false;
    }

    pub(super) fn nick(&self) -> &str {
        &self.nick
    }

    pub(super) const fn is_connected(&self) -> bool {
        self.connected
    }

    pub(super) const fn is_in_welcome_burst(&self) -> bool {
        self.welcome_deadline.is_some()
    }

    pub(super) fn update_nick(&mut self, nick: &str) {
        nick.clone_into(&mut self.nick);
    }

    fn complete(
        &mut self,
        config: &Config,
        caps: &Capabilities,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) {
        self.welcome_deadline = None;
        self.motd_lines = None;
        if let Some(expected) = config.selected_bouncer_network()
            && isupport.get("BOUNCER_NETID") != Advertised::Value(expected)
        {
            outbox.close(
                CloseKind::BouncerBindingFailed,
                "the bouncer did not confirm the selected network ID".to_owned(),
            );
            return;
        }
        self.connected = true;
        outbox.event(Event::Connected(Welcome {
            nick: self.origin.__nick(isupport.casemapping(), &self.nick),
            text: self.welcome_text.take().unwrap_or_default(),
            caps: caps.clone(),
            isupport: isupport.clone(),
        }));
    }
}

fn cap_is_unavailable(message: &LineRef<'_>) -> bool {
    message.command_is("410")
        || message.command_is("451")
        || message.command_is("001")
        || (message.command_is("421")
            && message
                .param(1)
                .is_some_and(|command| command.eq_ignore_ascii_case("CAP")))
}

fn registration_error(config: &Config) -> Option<BuildError> {
    Line::new("NICK")
        .param(&config.nick)
        .encode()
        .err()
        .or_else(|| {
            Line::new("USER")
                .param(&config.username)
                .param("0")
                .param("*")
                .param(&config.realname)
                .encode()
                .err()
        })
        .or_else(|| {
            config
                .alt_nicks
                .iter()
                .find_map(|nick| Line::new("NICK").param(nick).encode().err())
        })
}

fn rejects_nick(message: &LineRef<'_>) -> bool {
    ["432", "433", "436", "437"]
        .iter()
        .any(|numeric| message.command_is(numeric))
        || (message.command_is("400") && message.param(1) == Some("NICK"))
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
