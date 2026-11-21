use std::time::Duration;

use papo::{
    DisconnectCause, Disconnected, Error, Feature, ServerError, State, Unsupported,
    UnsupportedReason,
};

/// Builds an origin-free nickname for test input.
#[must_use]
pub fn nick(casemapping: papo::Casemapping, text: impl AsRef<str>) -> papo::Nick {
    papo::__testing::nickname(casemapping, text.as_ref())
}

/// Builds an origin-free channel name for test input.
#[must_use]
pub fn channel(casemapping: papo::Casemapping, text: impl AsRef<str>) -> papo::ChannelName {
    papo::__testing::channel_name(casemapping, text.as_ref())
}

/// Builds an origin-free account name for test input.
#[must_use]
pub fn account(casemapping: papo::Casemapping, text: impl AsRef<str>) -> papo::AccountName {
    papo::__testing::account_name(casemapping, text.as_ref())
}

/// Builds an origin-free mask for test input.
#[must_use]
pub fn mask(casemapping: papo::Casemapping, text: impl AsRef<str>) -> papo::Mask {
    papo::__testing::mask(casemapping, text.as_ref())
}

/// Builds a diagnostic for an unavailable IRC feature.
#[must_use]
pub const fn unsupported(feature: Feature, reason: UnsupportedReason) -> Unsupported {
    papo::__testing::unsupported(feature, reason)
}

/// Builds a minimal immutable client-state snapshot.
#[derive(Clone, Debug)]
pub struct StateBuilder {
    nick: String,
    connected: bool,
}

impl StateBuilder {
    /// Starts a disconnected snapshot using `nick`.
    #[must_use]
    pub fn new(nick: impl Into<String>) -> Self {
        Self {
            nick: nick.into(),
            connected: false,
        }
    }

    /// Sets whether the snapshot represents an open transport.
    #[must_use]
    pub const fn connected(mut self, connected: bool) -> Self {
        self.connected = connected;
        self
    }

    /// Finishes the snapshot.
    #[must_use]
    pub fn build(self) -> State {
        papo::__testing::state(self.nick, self.connected)
    }
}

/// Builds representative public client errors.
#[derive(Clone, Debug)]
pub struct ErrorBuilder(Error);

impl ErrorBuilder {
    /// Starts an operation-timeout error.
    #[must_use]
    pub const fn timeout() -> Self {
        Self(Error::Timeout)
    }

    /// Starts a disconnect error for the numbered next attempt and optional retry delay.
    #[must_use]
    pub const fn disconnected(
        cause: DisconnectCause,
        attempt: usize,
        reconnect_in: Option<Duration>,
    ) -> Self {
        Self(Error::Disconnected(Disconnected::new(
            cause,
            attempt,
            reconnect_in,
        )))
    }

    /// Starts a numeric server-refusal error.
    #[must_use]
    pub fn numeric(code: u16, params: Vec<String>) -> Self {
        Self(Error::Server(ServerError::from_numeric(code, params)))
    }

    /// Starts a standard-reply server-refusal error.
    #[must_use]
    pub fn standard(
        command: impl Into<papo::Command>,
        code: impl Into<papo::ReplyCode>,
        context: Vec<String>,
        description: impl Into<String>,
    ) -> Self {
        Self(Error::Server(
            ServerError::standard(command, code, description).context(context),
        ))
    }

    /// Finishes the error.
    #[must_use]
    pub fn build(self) -> Error {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use papo::{DisconnectCause, Error, Feature, Refusal, UnsupportedReason};

    use super::{ErrorBuilder, StateBuilder, unsupported};

    #[test]
    fn builds_state_and_distinct_error_categories() {
        let state = StateBuilder::new("alice").connected(true).build();
        assert_eq!(state.nick().as_str(), "alice");
        assert!(state.is_connected());

        assert!(matches!(ErrorBuilder::timeout().build(), Error::Timeout));
    }

    #[test]
    fn builds_disconnect_details() {
        let delay = Duration::from_secs(5);
        let Error::Disconnected(details) =
            ErrorBuilder::disconnected(DisconnectCause::PingTimeout, 2, Some(delay)).build()
        else {
            panic!("expected disconnect details");
        };
        assert_eq!(details.cause, DisconnectCause::PingTimeout);
        assert_eq!(details.attempt, 2);
        assert_eq!(details.reconnect_in, Some(delay));
    }

    #[test]
    fn builds_numeric_and_standard_refusals() {
        for (builder, numeric, command, code, description, params, refusal) in [
            (
                ErrorBuilder::numeric(403, vec!["#papo".into(), "No such channel".into()]),
                Some(403),
                None,
                "403",
                "No such channel",
                &["#papo", "No such channel"][..],
                Refusal::NoSuchChannel,
            ),
            (
                ErrorBuilder::standard(
                    "CHATHISTORY",
                    "INVALID_PARAMS",
                    vec!["LATEST".into(), "#papo".into()],
                    "Invalid parameters",
                ),
                None,
                Some("CHATHISTORY"),
                "INVALID_PARAMS",
                "Invalid parameters",
                &["LATEST", "#papo"][..],
                Refusal::InvalidParameters,
            ),
        ] {
            let Error::Server(error) = builder.build() else {
                panic!("expected server refusal");
            };
            assert_eq!(error.numeric(), numeric);
            assert_eq!(error.command().map(papo::Command::as_str), command);
            assert_eq!(error.code(), code);
            assert_eq!(error.description(), description);
            assert_eq!(error.params(), params);
            assert_eq!(error.refusal(), refusal);
        }
    }

    #[test]
    fn builds_unsupported_feature_details() {
        let details = unsupported(Feature::Monitor, UnsupportedReason::LimitZero);
        assert_eq!(details.feature, Feature::Monitor);
        assert_eq!(details.reason, UnsupportedReason::LimitZero);
    }
}
