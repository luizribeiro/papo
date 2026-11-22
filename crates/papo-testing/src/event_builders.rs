use papo::{
    Capabilities,
    event::{AwayChanged, Connected, Joined},
};

/// Builds capability facts for a synthetic connected event.
#[must_use]
pub fn capabilities(
    entries: impl IntoIterator<Item = (papo::Cap, Option<String>)>,
) -> Capabilities {
    papo::__testing::capabilities(entries)
}

/// Builds a connected event payload.
#[derive(Debug)]
pub struct ConnectedBuilder(Connected);

impl ConnectedBuilder {
    /// Starts a connected payload for `nick`.
    #[must_use]
    pub fn new(nick: impl Into<papo::Nick>) -> Self {
        Self(papo::__testing::connected(nick))
    }

    /// Sets the server's welcome text.
    #[must_use]
    pub fn welcome(mut self, welcome: impl Into<String>) -> Self {
        self.0.welcome = welcome.into();
        self
    }

    /// Sets the negotiated capabilities.
    #[must_use]
    pub fn capabilities(mut self, caps: Capabilities) -> Self {
        self.0.caps = caps;
        self
    }

    /// Marks the connection as a reconnect.
    #[must_use]
    pub const fn reconnected(mut self, reconnected: bool) -> Self {
        self.0.reconnected = reconnected;
        self
    }

    /// Finishes the payload.
    #[must_use]
    pub fn build(self) -> Connected {
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
        self.0.actor_party = if own {
            papo::Party::Me
        } else {
            papo::Party::Other
        };
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
        Self(papo::__testing::away_changed(
            nick,
            papo::AwayStatus::Present,
        ))
    }

    /// Sets the user's new presence state.
    #[must_use]
    pub fn away(mut self, away: papo::AwayStatus) -> Self {
        self.0.away = away;
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
    use super::{AwayChangedBuilder, ConnectedBuilder, JoinedBuilder};
    use crate::{capabilities, channel, nick};

    #[test]
    fn reads_capabilities_on_synthetic_connections() {
        let alice = nick(papo::Casemapping::default(), "alice");
        let connected = ConnectedBuilder::new(&alice)
            .capabilities(capabilities([
                (papo::cap::BATCH, Some(String::new())),
                (papo::cap::CHATHISTORY, None),
                (papo::cap::SASL, Some("PLAIN,SCRAM-SHA-256".to_owned())),
            ]))
            .build();
        let caps = &connected.caps;
        for (name, value) in [
            ("batch", Some(Some(""))),
            ("chathistory", Some(None)),
            ("draft/chathistory", Some(None)),
            ("sasl", Some(Some("PLAIN,SCRAM-SHA-256"))),
            ("server-time", None),
        ] {
            let name = papo::Cap::from(name);
            assert_eq!(caps.get(&name), value);
            assert_eq!(caps.contains(&name), value.is_some());
        }
        let expected = [
            ("batch", Some("")),
            ("draft/chathistory", None),
            ("sasl", Some("PLAIN,SCRAM-SHA-256")),
        ];
        assert!(
            caps.iter()
                .map(|(name, value)| (name.as_str(), value))
                .eq(expected)
        );
        let mut borrowed = Vec::new();
        for (name, value) in caps {
            borrowed.push((name.as_str(), value));
        }
        assert_eq!(borrowed, expected);
        let empty = ConnectedBuilder::new(alice).build();
        assert!(empty.caps.iter().next().is_none());
    }

    #[test]
    fn builds_common_event_payloads() {
        let mapping = papo::Casemapping::default();
        let connected = ConnectedBuilder::new(nick(mapping, "alice"))
            .welcome("Welcome to Ergo")
            .reconnected(true)
            .build();
        assert_eq!(connected.welcome, "Welcome to Ergo");
        assert!(connected.reconnected);

        let joined = JoinedBuilder::new(channel(mapping, "#papo"), nick(mapping, "alice"))
            .own(true)
            .build();
        assert_eq!(joined.channel.name().as_str(), "#papo");
        assert_eq!(joined.actor_party, papo::Party::Me);

        let away = AwayChangedBuilder::new(nick(mapping, "alice"))
            .away(papo::AwayStatus::__away_value(Some(
                "reviewing IRCv3".into(),
            )))
            .build();
        assert_eq!(
            away.away,
            papo::AwayStatus::__away_value(Some("reviewing IRCv3".into()))
        );
    }
}
