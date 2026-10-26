use std::time::Duration;

use papo::{DisconnectCause, Disconnected, Error, ServerError, State};

/// Builds an origin-free nickname for test input.
#[must_use]
pub fn nick(casemapping: papo::Casemapping, text: impl AsRef<str>) -> papo::Nick {
    casemapping.nick(text)
}

/// Builds an origin-free channel name for test input.
#[must_use]
pub fn channel(casemapping: papo::Casemapping, text: impl AsRef<str>) -> papo::ChannelName {
    casemapping.channel(text)
}

/// Builds an origin-free account name for test input.
#[must_use]
pub fn account(casemapping: papo::Casemapping, text: impl AsRef<str>) -> papo::AccountName {
    casemapping.account(text)
}

/// Builds an origin-free mask for test input.
#[must_use]
pub fn mask(casemapping: papo::Casemapping, text: impl AsRef<str>) -> papo::Mask {
    casemapping.mask(text)
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

    /// Starts a disconnect error with an optional retry delay.
    #[must_use]
    pub const fn disconnected(cause: DisconnectCause, reconnect_in: Option<Duration>) -> Self {
        Self(Error::Disconnected(Disconnected::new(cause, reconnect_in)))
    }

    /// Starts a numeric server-refusal error.
    #[must_use]
    pub fn numeric(code: u16, params: Vec<String>) -> Self {
        Self(Error::Server(ServerError::from_numeric(code, params)))
    }

    /// Starts a standard-reply server-refusal error.
    #[must_use]
    pub fn standard(
        command: impl Into<String>,
        code: impl Into<String>,
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
    use papo::{DisconnectCause, Error};

    use super::{ErrorBuilder, StateBuilder};

    #[test]
    fn builds_state_and_distinct_error_categories() {
        let state = StateBuilder::new("alice").connected(true).build();
        assert_eq!(state.nick().as_ref(), "alice");
        assert!(state.is_connected());

        assert!(matches!(ErrorBuilder::timeout().build(), Error::Timeout));
        assert!(matches!(
            ErrorBuilder::disconnected(DisconnectCause::PingTimeout, None).build(),
            Error::Disconnected(disconnected)
                if disconnected.cause == DisconnectCause::PingTimeout
        ));
        assert!(matches!(
            ErrorBuilder::numeric(403, vec!["alice".into(), "No such channel".into()]).build(),
            Error::Server(error) if error.code() == "403"
        ));
    }
}
