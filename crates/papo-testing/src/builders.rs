use std::time::SystemTime;

use papo::{AccountName, Message, MessageId, MessageKind, Origin};

/// Builds an incoming chat-message payload.
#[derive(Debug)]
pub struct MessageBuilder {
    message: Message,
    tags: Vec<(String, Option<String>)>,
}

impl MessageBuilder {
    /// Starts a live `PRIVMSG` with deterministic receipt time.
    #[must_use]
    pub fn new(target: impl Into<String>, text: impl Into<String>) -> Self {
        let target = target.into();
        let private = !target.starts_with(['#', '&']);
        Self {
            message: papo::__testing::message(None, target, text, private),
            tags: Vec::new(),
        }
    }

    /// Sets a user prefix with optional username and hostname.
    #[must_use]
    pub fn sender(
        mut self,
        nick: impl Into<papo::Nick>,
        user: Option<String>,
        host: Option<String>,
    ) -> Self {
        let nick = nick.into();
        if matches!(self.message.conversation, papo::Conversation::Private(_)) {
            self.message.conversation = papo::Conversation::Private(nick.clone());
        }
        self.message.sender = Some(papo::__testing::user_source(nick, user, host));
        self
    }

    /// Sets the semantic message kind.
    #[must_use]
    pub const fn kind(mut self, kind: MessageKind) -> Self {
        self.message.kind = kind;
        self
    }

    /// Sets the message origin.
    #[must_use]
    pub const fn origin(mut self, origin: Origin) -> Self {
        self.message.origin = origin;
        self
    }

    /// Sets the sender's authenticated account.
    #[must_use]
    pub fn account(mut self, account: impl Into<AccountName>) -> Self {
        self.message.account = Some(account.into());
        self
    }

    /// Sets the server-assigned message identifier.
    #[must_use]
    pub fn message_id(mut self, id: impl Into<MessageId>) -> Self {
        self.message.msgid = Some(id.into());
        self
    }

    /// Sets the raw wire text separately from the display text.
    #[must_use]
    pub fn raw_text(mut self, text: impl Into<String>) -> Self {
        self.message.raw_text = text.into();
        self
    }

    /// Sets local and optional server timestamps.
    #[must_use]
    pub const fn timestamps(
        mut self,
        received_at: SystemTime,
        server_time: Option<SystemTime>,
    ) -> Self {
        self.message.received_at = received_at;
        self.message.server_time = server_time;
        self
    }

    /// Appends an IRC message tag in wire order.
    #[must_use]
    pub fn tag(mut self, key: impl Into<String>, value: Option<String>) -> Self {
        self.tags.push((key.into(), value));
        self
    }

    /// Finishes the payload.
    #[must_use]
    pub fn build(mut self) -> Message {
        self.message.tags = papo::__testing::tags(self.tags);
        self.message
    }
}

#[cfg(test)]
mod tests {
    use papo::{MessageKind, Origin};

    use super::MessageBuilder;

    #[test]
    fn builds_private_message_details_and_ordered_tags() {
        let message = MessageBuilder::new("bob", "formatted hello")
            .sender(
                papo::Casemapping::default().nick("alice"),
                Some("a".into()),
                Some("cloak.example".into()),
            )
            .kind(MessageKind::Notice)
            .origin(Origin::Replay)
            .account(papo::Casemapping::default().account("alice"))
            .message_id("ergo-42")
            .raw_text("\u{2}formatted hello\u{2}")
            .tag("time", Some("2026-10-18T12:00:00.000Z".into()))
            .tag("bot", None)
            .build();

        assert_eq!(message.return_address(), "alice");
        assert_eq!(message.kind(), MessageKind::Notice);
        assert_eq!(message.origin(), Origin::Replay);
        assert_eq!(message.account(), Some("alice"));
        assert_eq!(message.msgid(), Some("ergo-42"));
        assert_eq!(message.tags().iter().count(), 2);
        assert_eq!(message.tags().get("bot"), Some(""));
    }
}
