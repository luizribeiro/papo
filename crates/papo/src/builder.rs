use std::time::Duration;

use papo_core::{Config, RateLimit, Sasl};

use crate::{AddressError, address::ServerAddress};

/// Configures an IRC client before it is built or connected.
#[must_use]
pub struct Builder {
    pub(crate) _address: Result<ServerAddress, AddressError>,
    pub(crate) config: Config,
    pub(crate) event_capacity: usize,
    username_set: bool,
    realname_set: bool,
}

impl Builder {
    pub(crate) fn new(address: &str) -> Self {
        Self {
            _address: address.parse(),
            config: Config::default(),
            event_capacity: crate::driver::DEFAULT_CHANNEL_CAPACITY,
            username_set: false,
            realname_set: false,
        }
    }

    /// Sets the primary nickname.
    pub fn nick(mut self, nick: impl Into<String>) -> Self {
        let nick = nick.into();
        self.config.nick.clone_from(&nick);
        if !self.username_set {
            self.config.username.clone_from(&nick);
        }
        if !self.realname_set {
            self.config.realname.clone_from(&nick);
        }
        self
    }

    /// Sets alternative nicknames to try after a collision.
    pub fn alt_nicks<I, S>(mut self, nicks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.alt_nicks = nicks.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the username sent during registration.
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.config.username = username.into();
        self.username_set = true;
        self
    }

    /// Sets the real name sent during registration.
    pub fn realname(mut self, realname: impl Into<String>) -> Self {
        self.config.realname = realname.into();
        self.realname_set = true;
        self
    }

    /// Sets the server password sent with `PASS`.
    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.config.password = Some(password.into());
        self
    }

    /// Configures SASL authentication.
    pub fn sasl(mut self, sasl: Sasl) -> Self {
        self.config.sasl = Some(sasl);
        self
    }

    /// Sets whether successful SASL authentication is required.
    pub fn require_sasl(mut self, required: bool) -> Self {
        self.config.require_sasl = required;
        self
    }

    /// Sets the Ergo multiclient device identifier.
    pub fn device(mut self, device: impl Into<String>) -> Self {
        self.config.device = Some(device.into());
        self
    }

    /// Replaces the capability names requested when advertised.
    pub fn caps<I, S>(mut self, caps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.wanted_caps = caps.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the outbound line rate limit.
    pub fn rate_limit(mut self, rate_limit: RateLimit) -> Self {
        self.config.rate_limit = rate_limit;
        self
    }

    /// Sets the idle interval before sending a keepalive probe.
    pub fn ping_interval(mut self, interval: Duration) -> Self {
        self.config.ping_interval = interval;
        self
    }

    /// Sets the maximum time allowed without an incoming line.
    pub fn ping_timeout(mut self, timeout: Duration) -> Self {
        self.config.ping_timeout = timeout;
        self
    }

    /// Sets the bounded event queue capacity.
    ///
    /// # Panics
    ///
    /// Panics when `capacity` is zero.
    pub fn event_capacity(mut self, capacity: usize) -> Self {
        assert!(capacity > 0, "event capacity must be positive");
        self.event_capacity = capacity;
        self
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use papo_core::{RateLimit, Sasl};

    use super::Builder;

    #[test]
    fn fills_every_session_configuration_field() {
        let builder = Builder::new("irc.example.net")
            .nick("ping-bot")
            .alt_nicks(["ping-bot_", "ping-bot__"])
            .username("ping")
            .realname("papo ping bot")
            .password("network password")
            .sasl(Sasl::plain("ping-bot", "correct horse battery staple"))
            .require_sasl(true)
            .device("phone")
            .caps(["message-tags", "sasl"]);
        let config = &builder.config;
        assert_eq!(config.nick, "ping-bot");
        assert_eq!(config.alt_nicks, ["ping-bot_", "ping-bot__"]);
        assert_eq!(config.username, "ping");
        assert_eq!(config.realname, "papo ping bot");
        assert_eq!(config.password.as_deref(), Some("network password"));
        assert!(config.sasl.is_some());
        assert!(config.require_sasl);
        assert_eq!(config.device.as_deref(), Some("phone"));
        assert_eq!(config.wanted_caps, ["message-tags", "sasl"]);
    }

    #[test]
    fn nick_supplies_only_implicit_username_and_realname_defaults() {
        let implicit = Builder::new("irc.example.net").nick("ping-bot").config;
        assert_eq!(
            (implicit.username.as_str(), implicit.realname.as_str()),
            ("ping-bot", "ping-bot")
        );
        let explicit = Builder::new("irc.example.net")
            .username("service")
            .realname("Papo service")
            .nick("ping-bot")
            .config;
        assert_eq!(
            (explicit.username.as_str(), explicit.realname.as_str()),
            ("service", "Papo service")
        );
    }

    #[test]
    fn configures_timing_rate_and_event_capacity() {
        let builder = Builder::new("irc.example.net")
            .rate_limit(RateLimit::unlimited())
            .ping_interval(Duration::from_secs(31))
            .ping_timeout(Duration::from_secs(93))
            .event_capacity(17);
        assert_eq!(builder.config.rate_limit, RateLimit::unlimited());
        assert_eq!(builder.config.ping_interval, Duration::from_secs(31));
        assert_eq!(builder.config.ping_timeout, Duration::from_secs(93));
        assert_eq!(builder.event_capacity, 17);
    }

    #[test]
    #[should_panic(expected = "event capacity must be positive")]
    fn rejects_an_empty_event_queue() {
        _ = Builder::new("irc.example.net").event_capacity(0);
    }
}
