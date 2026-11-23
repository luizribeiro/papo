use std::{collections::VecDeque, time::Duration};

use papo_proto::{Command, Line, LineRef, Param, Secret, WireLine};
use web_time::Instant;

use crate::{RequestError, Sasl, sasl::Mechanism};

/// The most secrets a session tracks for the lines the server echoes.
const MAX_SECRET_ECHOES: usize = 32;

/// What is shown in place of server text that repeats a secret we sent.
const REDACTED: &str = "<redacted>";

/// Secret parameters of sent lines the server repeats back, labelled or not.
///
/// A line from the server with a parameter equal to one is marked secret as it is read, before
/// any other part of the session copies it. Matching is by whole parameter, so a server that
/// rewrites the echoed text defeats it.
///
/// A secret stays tracked after it comes back, because a labelled echo, a response batch and
/// history may each repeat it. Only a secret that has come back, or whose echo is overdue (the
/// server refused the line), may make room for a newer one; a repeat after that is not marked.
#[derive(Debug)]
pub(crate) struct SecretEchoes {
    entries: VecDeque<SecretEcho>,
    echo_deadline: Duration,
}

#[derive(Debug)]
struct SecretEcho {
    secret: Param,
    sent_at: Instant,
    echoed: bool,
}

impl SecretEchoes {
    pub(crate) fn new(echo_deadline: Duration) -> Self {
        Self {
            entries: VecDeque::new(),
            echo_deadline,
        }
    }

    /// Remembers the secrets of a line the server will echo.
    ///
    /// Fails closed: when every tracked secret may still be echoed, the line is refused so that
    /// no echo can arrive untracked.
    pub(crate) fn track(&mut self, now: Instant, message: &Line) -> Result<(), RequestError> {
        let secrets = message
            .params()
            .iter()
            .filter(|param| param.__is_secret() && !param.is_empty())
            .collect::<Vec<_>>();
        if secrets.is_empty() {
            return Ok(());
        }
        let deadline = self.echo_deadline;
        let evictable = |entry: &SecretEcho| {
            entry.echoed || now.saturating_duration_since(entry.sent_at) >= deadline
        };
        let pending = self
            .entries
            .iter()
            .filter(|entry| !evictable(entry))
            .count();
        if pending + secrets.len() > MAX_SECRET_ECHOES {
            return Err(RequestError::PendingLimit);
        }
        let mut excess = (self.entries.len() + secrets.len()).saturating_sub(MAX_SECRET_ECHOES);
        self.entries.retain(|entry| {
            let evicted = excess > 0 && evictable(entry);
            excess -= usize::from(evicted);
            !evicted
        });
        self.entries
            .extend(secrets.into_iter().map(|secret| SecretEcho {
                secret: secret.clone(),
                sent_at: now,
                echoed: false,
            }));
        Ok(())
    }

    /// Lets the secrets of an earlier connection make room, while still marking their repeats.
    ///
    /// A new connection never echoes their lines, but a bouncer may replay them.
    pub(crate) fn handle_connected(&mut self) {
        for entry in &mut self.entries {
            entry.echoed = true;
        }
    }

    /// Returns `line`, or a copy of `message`, with every repeated secret marked secret.
    pub(crate) fn mark_echo(&mut self, line: Option<Line>, message: &LineRef<'_>) -> Option<Line> {
        let repeated = message
            .params()
            .enumerate()
            .filter(|(_, param)| self.observe(param))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if repeated.is_empty() {
            return line;
        }
        let mut line = line.unwrap_or_else(|| message.to_owned());
        for index in repeated {
            line.__mark_secret(index);
        }
        Some(line)
    }

    /// Returns server text for an event field, hiding it when it repeats a secret we sent.
    pub(crate) fn shown(&self, text: &str) -> String {
        if self.entries.iter().any(|entry| *entry.secret == *text) {
            REDACTED.to_owned()
        } else {
            text.to_owned()
        }
    }

    fn observe(&mut self, text: &str) -> bool {
        let mut repeated = false;
        for entry in self
            .entries
            .iter_mut()
            .filter(|entry| *entry.secret == *text)
        {
            entry.echoed = true;
            repeated = true;
        }
        repeated
    }
}

/// The peer can repeat a credential in a late or unlabelled reply. Keep the credentials for the
/// connection and refuse to send any beyond the bound.
#[derive(Debug, Default)]
pub(super) struct ReplySecrets(Vec<Secret>);

impl ReplySecrets {
    pub(super) fn track(&mut self, line: &WireLine, sasl: Option<&Sasl>) -> bool {
        const MAX_CREDENTIALS: usize = 256;
        let commands = [
            "MODE",
            "JOIN",
            "OPER",
            "PASS",
            "REGISTER",
            "VERIFY",
            "WEBIRC",
            "AUTHENTICATE",
        ];
        if !line.__has_secret_parameter() {
            return true;
        }
        let Ok(message) = LineRef::parse_bytes(line) else {
            return true;
        };
        if !commands.iter().any(|command| message.command_is(command)) {
            return true;
        }
        let decoded = sasl
            .filter(|_| message.command_is("AUTHENTICATE"))
            .into_iter()
            .flat_map(|sasl| &sasl.mechanisms)
            .filter_map(|mechanism| match mechanism {
                Mechanism::Plain { password, .. } => Some(password.expose()),
                Mechanism::OAuthBearer { token } => Some(token.expose()),
                _ => None,
            });
        let values = papo_proto::__wire_secret_parameters(line).chain(decoded);
        for value in values.filter(|value| !value.is_empty()) {
            if self.0.iter().any(|secret| secret.expose() == value) {
                continue;
            }
            if self.0.len() == MAX_CREDENTIALS {
                return false;
            }
            self.0.push(Secret::new(value));
        }
        true
    }

    pub(super) fn repeated_in_bytes(&self, line: &[u8]) -> bool {
        self.0.iter().any(|secret| {
            let value = secret.expose().as_bytes();
            line.windows(value.len()).any(|bytes| bytes == value)
        })
    }

    pub(super) fn redact(&self, message: &LineRef<'_>) -> Option<Line> {
        if self.0.is_empty()
            || !(matches!(message.command(), Command::Numeric(_))
                || [
                    "FAIL",
                    "WARN",
                    "NOTE",
                    "ERROR",
                    "NOTICE",
                    "REGISTER",
                    "VERIFY",
                    "AUTHENTICATE",
                ]
                .iter()
                .any(|command| message.command_is(command)))
        {
            return None;
        }
        let mut line = None;
        let last = message.len().saturating_sub(1);
        for (index, param) in message.params().enumerate() {
            if self.0.iter().any(|secret| {
                let value = secret.expose();
                param == value || index == last && param.contains(value)
            }) {
                let line = line.get_or_insert_with(|| message.to_owned());
                papo_proto::__mask_param(line, index);
            }
        }
        line
    }
}
