//! Helpers for parsing and responding to IRC bot commands.

use crate::{Event, MessageEvent, MessageKind};

/// Command parsing and reply policy shared by bot helpers.
#[derive(Clone, Debug)]
pub struct BotConfig {
    prefix: String,
    mention: bool,
}

impl BotConfig {
    /// Creates bot configuration with the given command prefix.
    #[must_use]
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            mention: true,
        }
    }

    /// Enables or disables commands addressed to the client's nickname.
    #[must_use]
    pub const fn mention(mut self, enabled: bool) -> Self {
        self.mention = enabled;
        self
    }
}

/// A parsed command from a live private message.
#[derive(Clone, Debug)]
pub struct Command {
    message: MessageEvent,
    name: String,
    args: Vec<String>,
}

impl Command {
    /// Parses a live `PRIVMSG` beginning with the configured prefix or bot mention.
    #[must_use]
    pub fn parse(event: &Event, config: &BotConfig) -> Option<Self> {
        let Event::Message(message) = event else {
            return None;
        };
        if !message.is_live() || message.kind() != MessageKind::Privmsg {
            return None;
        }
        let body = addressed_body(message, config)?;
        let mut words = split_args(body)?;
        if words.is_empty() {
            return None;
        }
        let name = words.remove(0).to_ascii_lowercase();
        Some(Self {
            message: message.clone(),
            name,
            args: words,
        })
    }

    /// Returns the ASCII-lowercase command name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the parsed command arguments.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Returns the message that contained this command.
    #[must_use]
    pub const fn message(&self) -> &MessageEvent {
        &self.message
    }
}

fn addressed_body<'a>(message: &'a MessageEvent, config: &BotConfig) -> Option<&'a str> {
    if let Some(body) = message.text().strip_prefix(&config.prefix) {
        return Some(body.trim_start());
    }
    if !config.mention {
        return None;
    }
    let boundary = message.text().find([':', ','])?;
    let (address, body) = message.text().split_at(boundary);
    message
        .casemapping()
        .eq(address.trim(), &message.client_nick())
        .then(|| body[1..].trim_start())
}

fn split_args(input: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut started = false;
    for character in input.chars() {
        if escaped {
            word.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
            started = true;
        } else if quote == Some(character) {
            quote = None;
        } else if quote.is_none() && matches!(character, '\'' | '"') {
            quote = Some(character);
            started = true;
        } else if quote.is_none() && character.is_whitespace() {
            if started {
                words.push(std::mem::take(&mut word));
                started = false;
            }
        } else {
            word.push(character);
            started = true;
        }
    }
    if quote.is_some() || escaped {
        return None;
    }
    if started {
        words.push(word);
    }
    Some(words)
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use papo_core::Capabilities;
    use papo_proto::{Isupport, Message};

    use super::{BotConfig, Command, Event, MessageEvent};

    fn event_as(command: &str, text: &str, nick: &str, source: &str) -> Event {
        Event::Message(
            MessageEvent::new(
                Message::new(command, ["#papo", text]).with_source(source),
                &Isupport::default(),
                nick,
                crate::Client::disconnected_for_test(),
                &Capabilities::new(),
                UNIX_EPOCH,
            )
            .unwrap(),
        )
    }

    fn event(text: &str) -> Event {
        event_as("PRIVMSG", text, "papo", "emily!~emily@localhost")
    }

    #[test]
    fn parses_prefixed_live_privmsg() {
        let command = Command::parse(&event("!EcHo hello world"), &BotConfig::new("!")).unwrap();
        assert_eq!(command.name(), "echo");
        assert_eq!(command.args(), ["hello", "world"]);
        assert_eq!(command.message().text(), "!EcHo hello world");
    }

    #[test]
    fn parses_mentions_quotes_escapes_and_empty_arguments() {
        let event = event_as(
            "PRIVMSG",
            "PAPO: EcHo 'Ergo Chat' one\\ two \"\"",
            "papo",
            "emily!~emily@localhost",
        );
        let command = Command::parse(&event, &BotConfig::new("!")).unwrap();
        assert_eq!(command.name(), "echo");
        assert_eq!(command.args(), ["Ergo Chat", "one two", ""]);
    }

    #[test]
    fn rejects_unaddressed_non_live_non_privmsg_and_incomplete_quotes() {
        let config = BotConfig::new("!").mention(false);
        assert!(Command::parse(&event("papo: ping"), &config).is_none());
        assert!(Command::parse(&event("!echo 'unfinished"), &config).is_none());
        assert!(
            Command::parse(&event_as("NOTICE", "!ping", "papo", "emily!u@h"), &config).is_none()
        );
        assert!(
            Command::parse(&event_as("PRIVMSG", "!ping", "papo", "papo!u@h"), &config).is_none()
        );
    }
}
