use papo::{
    Capabilities,
    event::{AwayChanged, Joined, Registered},
};

/// Builds a registration event payload.
#[derive(Debug)]
pub struct RegisteredBuilder(Registered);

impl RegisteredBuilder {
    /// Starts a registration payload for `nick`.
    #[must_use]
    pub fn new(nick: impl Into<papo::Nick>) -> Self {
        Self(papo::__testing::registered(nick))
    }

    /// Sets the server's welcome text.
    #[must_use]
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.0.text = text.into();
        self
    }

    /// Sets the negotiated capabilities.
    #[must_use]
    pub fn capabilities(mut self, caps: Capabilities) -> Self {
        self.0.caps = caps;
        self
    }

    /// Marks registration as a reconnect.
    #[must_use]
    pub const fn reconnected(mut self, reconnected: bool) -> Self {
        self.0.reconnected = reconnected;
        self
    }

    /// Finishes the payload.
    #[must_use]
    pub fn build(self) -> Registered {
        self.0
    }
}

/// Builds a channel-join event payload.
#[derive(Debug)]
pub struct JoinedBuilder(Joined);

impl JoinedBuilder {
    /// Starts a join payload.
    #[must_use]
    pub fn new(channel: impl Into<papo::ChannelName>, nick: impl Into<papo::Nick>) -> Self {
        Self(papo::__testing::joined(channel, nick, false))
    }

    /// Marks whether the joining user is this client.
    #[must_use]
    pub fn own(mut self, own: bool) -> Self {
        self.0.is_self = own;
        self
    }

    /// Finishes the payload.
    #[must_use]
    pub fn build(self) -> Joined {
        self.0
    }
}

/// Builds an away-state event payload.
#[derive(Debug)]
pub struct AwayChangedBuilder(AwayChanged);

impl AwayChangedBuilder {
    /// Starts an away-state payload for `nick`.
    #[must_use]
    pub fn new(nick: impl Into<papo::Nick>) -> Self {
        Self(papo::__testing::away_changed(nick, None))
    }

    /// Sets the away message, or clears it for a return notification.
    #[must_use]
    pub fn message(mut self, message: Option<impl Into<String>>) -> Self {
        self.0.message = message.map(Into::into);
        self
    }

    /// Finishes the payload.
    #[must_use]
    pub fn build(self) -> AwayChanged {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{AwayChangedBuilder, JoinedBuilder, RegisteredBuilder};

    #[test]
    fn builds_common_event_payloads() {
        let mapping = papo::Casemapping::default();
        let registered = RegisteredBuilder::new(mapping.nick("alice"))
            .text("Welcome to Ergo")
            .reconnected(true)
            .build();
        assert_eq!(registered.text, "Welcome to Ergo");
        assert!(registered.reconnected);

        let joined = JoinedBuilder::new(mapping.channel("#papo"), mapping.nick("alice"))
            .own(true)
            .build();
        assert_eq!(joined.channel.as_ref(), "#papo");
        assert!(joined.is_self);

        let away = AwayChangedBuilder::new(mapping.nick("alice"))
            .message(Some("reviewing IRCv3"))
            .build();
        assert_eq!(away.message.as_deref(), Some("reviewing IRCv3"));
    }
}
