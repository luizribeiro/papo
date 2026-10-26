use std::time::SystemTime;

use papo::{Account, AccountName, Message, MessageId, MessageKind};

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
    pub fn actor(
        mut self,
        nick: impl Into<papo::Nick>,
        username: Option<String>,
        host: Option<String>,
    ) -> Self {
        let nick = nick.into();
        if matches!(self.message.conversation, papo::Conversation::Private(_)) {
            self.message.conversation = papo::Conversation::Private(nick.clone());
        }
        self.message.actor = papo::__testing::user_source(nick, username, host);
        self
    }

    /// Sets the semantic message kind.
    #[must_use]
    pub const fn kind(mut self, kind: MessageKind) -> Self {
        self.message.kind = kind;
        self
    }

    /// Sets the actor's authenticated account.
    #[must_use]
    pub fn account(mut self, account: impl Into<AccountName>) -> Self {
        papo::__testing::set_message_account(&mut self.message, Account::LoggedIn(account.into()));
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
        self.message.server_time = server_time;
        self.message.received_at = received_at;
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
    use papo::MessageKind;

    use super::MessageBuilder;

    #[test]
    fn builds_private_message_details_and_ordered_tags() {
        let message = MessageBuilder::new("bob", "formatted hello")
            .actor(
                crate::nick(papo::Casemapping::default(), "alice"),
                Some("a".into()),
                Some("cloak.example".into()),
            )
            .kind(MessageKind::Text)
            .account(crate::account(papo::Casemapping::default(), "alice"))
            .message_id("ergo-42")
            .raw_text("\u{2}formatted hello\u{2}")
            .tag("time", Some("2026-10-18T12:00:00.000Z".into()))
            .tag("bot", None)
            .build();

        assert_eq!(message.return_address(), "alice");
        assert_eq!(message.kind, MessageKind::Text);
        assert!(
            matches!(message.actor.account(), papo::Account::LoggedIn(account) if account == "alice")
        );
        assert_eq!(message.msgid.as_deref(), Some("ergo-42"));
        assert_eq!(message.tags.iter().count(), 2);
        assert_eq!(message.tags.get("bot"), Some(""));
    }
}
