use papo_core::{Capabilities, StateLimit, Welcome, sts::StsAction};
use papo_proto::{Ctcp, Isupport, Message, Source, strip_formatting};

/// The semantic kind of an incoming chat message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MessageKind<'a> {
    /// A regular `PRIVMSG`.
    Privmsg,
    /// A regular `NOTICE`.
    Notice,
    /// A CTCP `ACTION`.
    Action,
    /// Another CTCP command.
    Ctcp(&'a str),
}

/// An incoming `PRIVMSG` or `NOTICE` with connection context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageEvent {
    raw: Message,
    text: String,
    kind: StoredKind,
    isupport: Isupport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StoredKind {
    Privmsg,
    Notice,
    Action,
    Ctcp(String),
}

impl MessageEvent {
    pub(crate) fn new(raw: Message, isupport: &Isupport) -> Result<Self, Message> {
        let message = raw.as_ref();
        let Some(raw_text) = message.param(0).and_then(|_| message.param(1)) else {
            return Err(raw);
        };
        let (kind, displayed) = match (message.raw_command(), Ctcp::parse(raw_text)) {
            (_, Some(ctcp)) if ctcp.command.eq_ignore_ascii_case("ACTION") => {
                (StoredKind::Action, ctcp.params.unwrap_or(""))
            }
            (_, Some(ctcp)) => (StoredKind::Ctcp(ctcp.command.to_owned()), raw_text),
            (command, None) if command.eq_ignore_ascii_case("PRIVMSG") => {
                (StoredKind::Privmsg, raw_text)
            }
            (command, None) if command.eq_ignore_ascii_case("NOTICE") => {
                (StoredKind::Notice, raw_text)
            }
            _ => return Err(raw),
        };
        let text = strip_formatting(displayed).into_owned();
        Ok(Self {
            raw,
            text,
            kind,
            isupport: isupport.clone(),
        })
    }

    /// Returns the semantic kind of this message.
    #[must_use]
    pub fn kind(&self) -> MessageKind<'_> {
        match &self.kind {
            StoredKind::Privmsg => MessageKind::Privmsg,
            StoredKind::Notice => MessageKind::Notice,
            StoredKind::Action => MessageKind::Action,
            StoredKind::Ctcp(command) => MessageKind::Ctcp(command),
        }
    }

    /// Returns the display text with IRC formatting removed.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the text parameter exactly as received.
    #[must_use]
    pub fn raw_text(&self) -> &str {
        self.raw.as_ref().param(1).unwrap_or("")
    }

    /// Returns the parsed source, when the server supplied one.
    #[must_use]
    pub fn source(&self) -> Option<Source<'_>> {
        self.raw.as_ref().source()
    }

    /// Returns the message target.
    #[must_use]
    pub fn target(&self) -> &str {
        self.raw.as_ref().param(0).unwrap_or("")
    }

    /// Reports whether the target is not a channel under the captured `CHANTYPES` value.
    #[must_use]
    pub fn is_private(&self) -> bool {
        !self.isupport.is_channel(self.target())
    }

    /// Returns the channel target, or the sender for a private message.
    #[must_use]
    pub fn reply_target(&self) -> &str {
        if self.is_private()
            && let Some(Source::User { nick, .. }) = self.source()
        {
            nick
        } else {
            self.target()
        }
    }

    /// Borrows the underlying IRC message.
    #[must_use]
    pub fn as_ref(&self) -> papo_proto::MessageRef<'_> {
        self.raw.as_ref()
    }
}

/// An event emitted by an IRC client.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// Connection registration completed.
    Registered(Welcome),
    /// A replacement connection completed registration.
    Reconnected(Welcome),
    /// A parsed message not consumed by protocol behavior.
    Message(MessageEvent),
    /// A parsed message without a typed event variant.
    Raw(Message),
    /// A line from the server could not be decoded or parsed.
    Malformed {
        /// The original bytes, excluding the line ending.
        line: Vec<u8>,
    },
    /// Configured authentication failed and registration is continuing.
    AuthenticationFailed {
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
    },
    /// The capabilities enabled on an established connection changed.
    CapsChanged {
        /// Newly enabled capabilities and updated advertised values.
        added: Capabilities,
        /// Capabilities removed by the server.
        removed: Capabilities,
    },
    /// A plaintext connection must be replaced with TLS on `port`.
    StsUpgrade {
        /// The advertised TLS port.
        port: u16,
    },
    /// An STS persistence action for the client to store.
    StsPolicy(StsAction),
    /// A user joined a channel shared with the client.
    Joined {
        /// The joined channel.
        channel: String,
        /// The joining user's nickname.
        nick: String,
    },
    /// Initial channel membership synchronization completed.
    ChannelSynced {
        /// The synchronized channel.
        channel: String,
    },
    /// A user left a shared channel with `PART`.
    Parted {
        /// The departed channel.
        channel: String,
        /// The departing user's nickname.
        nick: String,
        /// The optional part reason.
        reason: Option<String>,
    },
    /// A user was removed from a shared channel with `KICK`.
    Kicked {
        /// The affected channel.
        channel: String,
        /// The removed user's nickname.
        nick: String,
        /// The user or server that issued the kick.
        by: String,
        /// The optional kick reason.
        reason: Option<String>,
    },
    /// A user sharing channels with the client disconnected.
    Quit {
        /// The disconnected user's nickname.
        nick: String,
        /// The optional quit reason.
        reason: Option<String>,
        /// Shared channels captured before the user was removed.
        channels: Vec<String>,
    },
    /// A user changed nickname.
    NickChanged {
        /// The previous nickname.
        old: String,
        /// The new nickname.
        new: String,
    },
    /// Server input exceeded a configured state-retention bound.
    StateLimitReached {
        /// The state collection that rejected another entry.
        limit: StateLimit,
    },
    /// The connection ended.
    Disconnected {
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
        /// Whether the client will try to connect again.
        will_retry: bool,
    },
}

pub(crate) enum CoreEvent {
    Forward(Event),
    Close(String),
    Unknown,
}

pub(crate) fn from_core(event: papo_core::Event) -> CoreEvent {
    CoreEvent::Forward(match event {
        papo_core::Event::Registered(welcome) => Event::Registered(welcome),
        papo_core::Event::Message(raw) => Event::Raw(raw),
        papo_core::Event::Malformed { line } => Event::Malformed { line },
        papo_core::Event::Close { reason } => return CoreEvent::Close(reason),
        papo_core::Event::AuthenticationFailed { reason } => Event::AuthenticationFailed { reason },
        papo_core::Event::CapsChanged { added, removed } => Event::CapsChanged { added, removed },
        papo_core::Event::StsUpgrade { port } => Event::StsUpgrade { port },
        papo_core::Event::StsPolicy(action) => Event::StsPolicy(action),
        papo_core::Event::Joined { channel, nick } => Event::Joined { channel, nick },
        papo_core::Event::ChannelSynced { channel } => Event::ChannelSynced { channel },
        papo_core::Event::Parted {
            channel,
            nick,
            reason,
        } => Event::Parted {
            channel,
            nick,
            reason,
        },
        papo_core::Event::Kicked {
            channel,
            nick,
            by,
            reason,
        } => Event::Kicked {
            channel,
            nick,
            by,
            reason,
        },
        papo_core::Event::Quit {
            nick,
            reason,
            channels,
        } => Event::Quit {
            nick,
            reason,
            channels,
        },
        papo_core::Event::NickChanged { old, new } => Event::NickChanged { old, new },
        papo_core::Event::StateLimitReached { limit } => Event::StateLimitReached { limit },
        _ => return CoreEvent::Unknown,
    })
}

#[cfg(test)]
mod tests {
    use super::{CoreEvent, Event, MessageEvent, MessageKind, from_core};
    use papo_core::StateLimit;
    use papo_proto::{Isupport, Message, Source};

    fn chat(command: &str, text: &str) -> MessageEvent {
        MessageEvent::new(
            Message::new(command, ["#papo", text]).with_source("emily!~emily@localhost"),
            &Isupport::default(),
        )
        .unwrap()
    }

    #[test]
    fn classifies_chat_and_ctcp_messages() {
        assert_eq!(chat("PRIVMSG", "hello").kind(), MessageKind::Privmsg);
        assert_eq!(chat("NOTICE", "hello").kind(), MessageKind::Notice);
        assert_eq!(
            chat("PRIVMSG", "\x01ACTION waves\x01").kind(),
            MessageKind::Action
        );
        assert_eq!(
            chat("PRIVMSG", "\x01VERSION\x01").kind(),
            MessageKind::Ctcp("VERSION")
        );
    }

    #[test]
    fn exposes_raw_and_display_text_once() {
        let formatted = chat("PRIVMSG", "hello \x02from Ergo\x02");
        assert_eq!(formatted.raw_text(), "hello \x02from Ergo\x02");
        assert_eq!(formatted.text(), "hello from Ergo");
        let action = chat("PRIVMSG", "\x01ACTION writes the specification\x01");
        assert_eq!(action.raw_text(), "\x01ACTION writes the specification\x01");
        assert_eq!(action.text(), "writes the specification");
    }

    #[test]
    fn routes_channels_and_private_messages_with_ergo_chantypes() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CHANTYPES=#"]);
        let channel = MessageEvent::new(
            Message::privmsg("#papo", "hello").with_source("emily!~emily@localhost"),
            &isupport,
        )
        .unwrap();
        assert!(!channel.is_private());
        assert_eq!(channel.target(), "#papo");
        assert_eq!(channel.reply_target(), "#papo");
        assert_eq!(
            channel.source(),
            Some(Source::User {
                nick: "emily",
                user: Some("~emily"),
                host: Some("localhost")
            })
        );

        let private = MessageEvent::new(
            Message::privmsg("papo", "hello").with_source("emily!~emily@localhost"),
            &isupport,
        )
        .unwrap();
        assert!(private.is_private());
        assert_eq!(private.reply_target(), "emily");
    }

    fn forwarded(event: papo_core::Event) -> Event {
        let CoreEvent::Forward(event) = from_core(event) else {
            panic!("event was not forwarded")
        };
        event
    }

    #[test]
    fn maps_joined() {
        assert_eq!(
            forwarded(papo_core::Event::Joined {
                channel: "#papo".into(),
                nick: "emily".into()
            }),
            Event::Joined {
                channel: "#papo".into(),
                nick: "emily".into()
            }
        );
    }

    #[test]
    fn maps_channel_synced() {
        assert_eq!(
            forwarded(papo_core::Event::ChannelSynced {
                channel: "#papo".into()
            }),
            Event::ChannelSynced {
                channel: "#papo".into()
            }
        );
    }

    #[test]
    fn maps_parted() {
        assert_eq!(
            forwarded(papo_core::Event::Parted {
                channel: "#papo".into(),
                nick: "emily".into(),
                reason: Some("Leaving for lunch".into()),
            }),
            Event::Parted {
                channel: "#papo".into(),
                nick: "emily".into(),
                reason: Some("Leaving for lunch".into()),
            }
        );
    }

    #[test]
    fn maps_kicked() {
        assert_eq!(
            forwarded(papo_core::Event::Kicked {
                channel: "#papo".into(),
                nick: "emily".into(),
                by: "alice".into(),
                reason: Some("Flooding".into()),
            }),
            Event::Kicked {
                channel: "#papo".into(),
                nick: "emily".into(),
                by: "alice".into(),
                reason: Some("Flooding".into()),
            }
        );
    }

    #[test]
    fn maps_quit() {
        assert_eq!(
            forwarded(papo_core::Event::Quit {
                nick: "emily".into(),
                reason: Some("Client exited".into()),
                channels: vec!["#papo".into(), "#rust".into()],
            }),
            Event::Quit {
                nick: "emily".into(),
                reason: Some("Client exited".into()),
                channels: vec!["#papo".into(), "#rust".into()],
            }
        );
    }

    #[test]
    fn maps_nick_changed() {
        assert_eq!(
            forwarded(papo_core::Event::NickChanged {
                old: "emily".into(),
                new: "em".into(),
            }),
            Event::NickChanged {
                old: "emily".into(),
                new: "em".into(),
            }
        );
    }

    #[test]
    fn maps_state_limit_reached() {
        assert_eq!(
            forwarded(papo_core::Event::StateLimitReached {
                limit: StateLimit::Members
            }),
            Event::StateLimitReached {
                limit: StateLimit::Members
            }
        );
    }
}
