use std::{collections::VecDeque, time::Duration};

use papo_proto::{Command, Isupport, Line, LineRef, Param, Secret, WireLine};
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
        let mut new = Vec::new();
        for value in values.filter(|value| !value.is_empty()) {
            if self.contains(value) || new.contains(&value) {
                continue;
            }
            if self.0.len() + new.len() == MAX_CREDENTIALS {
                return false;
            }
            new.push(value);
        }
        self.0.extend(new.into_iter().map(Secret::new));
        true
    }

    pub(super) fn contains(&self, value: &str) -> bool {
        self.0.iter().any(|secret| secret.expose() == value)
    }

    /// Reports replacement parameters before decoding can change their bytes.
    pub(super) fn redact(
        &self,
        mut input: &[u8],
        nick: &str,
        isupport: &Isupport,
        mut replace: impl FnMut(usize, &[u8], Vec<u8>),
    ) {
        if self.0.is_empty() {
            return;
        }
        for prefix in *b"@:" {
            if trim_spaces(input).starts_with(&[prefix]) {
                next_word(&mut input);
            }
        }
        let command = std::str::from_utf8(next_word(&mut input)).unwrap_or_default();
        let command = Command::parse(command);
        let channel_index = match command {
            Command::Numeric(numeric) if crate::is_failure_numeric(numeric) => {
                numeric_channel_parameter(numeric)
            }
            Command::Fail | Command::Warn | Command::Note => standard_channel_parameter(input),
            Command::Error => None,
            _ => return,
        };

        let mut params = std::iter::from_fn(|| {
            let remaining = trim_spaces(input);
            if let Some(trailing) = remaining.strip_prefix(b":") {
                input = b"";
                Some(trailing)
            } else {
                (!remaining.is_empty()).then(|| next_word(&mut input))
            }
        })
        .enumerate()
        .peekable();
        while let Some((index, param)) = params.next() {
            let last = params.peek().is_none();
            if is_reply_address(command, channel_index, index, last, param, nick, isupport) {
                continue;
            }
            let mut masked = Vec::new();
            let mut copied = 0;
            for range in self.matches(param) {
                masked.extend_from_slice(param.get(copied..range.start).unwrap_or_default());
                masked.extend_from_slice(REDACTED.as_bytes());
                copied = range.end;
            }
            if copied != 0 {
                masked.extend_from_slice(param.get(copied..).unwrap_or_default());
                replace(index, param, masked);
            }
        }
    }
}

fn trim_spaces(mut input: &[u8]) -> &[u8] {
    while let Some((&b' ', rest)) = input.split_first() {
        input = rest;
    }
    input
}

fn next_word<'a>(input: &mut &'a [u8]) -> &'a [u8] {
    let mut fields = trim_spaces(input).splitn(2, |&byte| byte == b' ');
    let word = fields.next().unwrap_or_default();
    *input = fields.next().unwrap_or_default();
    word
}

fn numeric_channel_parameter(numeric: papo_proto::Numeric) -> Option<usize> {
    match numeric.code() {
        441 | 443 => Some(2),
        403..=405 | 437 | 442 | 470..=471 | 473..=479 | 482 | 489 | 525 | 696..=698 => Some(1),
        _ => None,
    }
}

fn standard_channel_parameter(mut params: &[u8]) -> Option<usize> {
    let command = std::str::from_utf8(next_word(&mut params)).unwrap_or_default();
    matches!(
        Command::parse(command),
        Command::Invite
            | Command::Join
            | Command::Kick
            | Command::List
            | Command::Mode
            | Command::Names
            | Command::Notice
            | Command::Part
            | Command::Privmsg
            | Command::Topic
            | Command::Who
            | Command::Tagmsg
            | Command::Chathistory
            | Command::Redact
            | Command::Metadata
    )
    .then_some(2)
}

fn is_reply_address(
    command: Command<'_>,
    channel_index: Option<usize>,
    index: usize,
    last: bool,
    param: &[u8],
    nick: &str,
    isupport: &Isupport,
) -> bool {
    let Ok(param) = std::str::from_utf8(param) else {
        return false;
    };
    (matches!(command, Command::Numeric(_))
        && index == 0
        && crate::names_equal(isupport.casemapping(), param, nick))
        || (!last && channel_index == Some(index) && isupport.is_channel(param))
}

impl ReplySecrets {
    fn matches<'a>(&'a self, param: &'a [u8]) -> impl Iterator<Item = std::ops::Range<usize>> + 'a {
        let mut exact = self
            .0
            .iter()
            .any(|secret| credential_prefix(param, secret.expose()) == Some(param.len()));
        let mut remaining = param;
        std::iter::from_fn(move || {
            if std::mem::take(&mut exact) {
                remaining = b"";
                return Some(0..param.len());
            }
            while !remaining.is_empty() {
                let offset = param.len() - remaining.len();
                let mut words = remaining.splitn(2, u8::is_ascii_whitespace);
                let word = words.next()?;
                let equals = word.iter().position(|&byte| byte == b'=');
                // Credentials can contain assignment delimiters, such as base64 padding.
                let range = if let Some(range) = self.token(remaining, word.len())
                    && equals.is_none_or(|index| range.contains(&index))
                {
                    let suffix = remaining.get(range.end..)?;
                    let mut suffix = suffix.splitn(2, u8::is_ascii_whitespace);
                    remaining = suffix.nth(1).unwrap_or_default();
                    Some(range)
                } else if let Some(equals) = equals {
                    let range = self.token(remaining.get(..equals)?, equals);
                    remaining = remaining.get(equals + 1..)?;
                    range
                } else {
                    remaining = words.next().unwrap_or_default();
                    None
                };
                if let Some(range) = range {
                    return Some(offset + range.start..offset + range.end);
                }
            }
            None
        })
    }

    fn token(&self, token: &[u8], first_word_len: usize) -> Option<std::ops::Range<usize>> {
        self.0
            .iter()
            .filter_map(|secret| {
                let value = secret.expose();
                let punctuation = token
                    .iter()
                    .take_while(|byte| byte.is_ascii_punctuation())
                    .count();
                (0..=punctuation).take(first_word_len).find_map(|start| {
                    let length = credential_prefix(token.get(start..)?, value)?;
                    let end = start + length;
                    token
                        .get(end..)?
                        .iter()
                        .take_while(|byte| !byte.is_ascii_whitespace())
                        .all(u8::is_ascii_punctuation)
                        .then_some(start..end)
                })
            })
            .max_by_key(std::ops::Range::len)
    }
}

fn credential_prefix(bytes: &[u8], value: &str) -> Option<usize> {
    if bytes.starts_with(value.as_bytes()) {
        return Some(value.len());
    }
    if value.is_ascii() {
        return None;
    }
    let length = value.chars().count();
    for decode in [char::from as fn(u8) -> char, windows_1252] {
        if let Some(prefix) = bytes.get(..length)
            && value.chars().eq(prefix.iter().copied().map(decode))
        {
            return Some(length);
        }
    }
    None
}

fn windows_1252(byte: u8) -> char {
    const C1: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    C1.get(usize::from(byte.wrapping_sub(0x80)))
        .copied()
        .unwrap_or_else(|| char::from(byte))
}
