//! Helpers for parsing and responding to IRC bot commands.

use crate::{Event, MessageEvent, MessageKind};

/// Command parsing and reply policy shared by bot helpers.
#[derive(Clone, Debug)]
pub struct BotConfig {
    prefix: String,
}

impl BotConfig {
    /// Creates bot configuration with the given command prefix.
    #[must_use]
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
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
        let body = message.text().strip_prefix(&config.prefix)?.trim_start();
        let mut words: Vec<_> = body.split_whitespace().map(str::to_owned).collect();
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

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use papo_core::Capabilities;
    use papo_proto::{Isupport, Message};

    use super::{BotConfig, Command, Event, MessageEvent};

    fn event(text: &str) -> Event {
        Event::Message(
            MessageEvent::new(
                Message::privmsg("#papo", text).with_source("emily!~emily@localhost"),
                &Isupport::default(),
                "papo",
                crate::Client::disconnected_for_test(),
                &Capabilities::new(),
                UNIX_EPOCH,
            )
            .unwrap(),
        )
    }

    #[test]
    fn parses_prefixed_live_privmsg() {
        let command = Command::parse(&event("!EcHo hello world"), &BotConfig::new("!")).unwrap();
        assert_eq!(command.name(), "echo");
        assert_eq!(command.args(), ["hello", "world"]);
        assert_eq!(command.message().text(), "!EcHo hello world");
    }
}
