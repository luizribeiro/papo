use std::fmt;

use papo::Event;

/// A failure while arranging or observing a test fixture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureError {
    message: String,
}

impl FixtureError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Creates a failure with bounded event diagnostics.
    #[doc(hidden)]
    pub fn with_events(message: impl fmt::Display, events: &[Event]) -> Self {
        Self::new(format!("{message}; last events: {events:#?}"))
    }
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FixtureError {}

impl From<papo::Error> for FixtureError {
    fn from(error: papo::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<std::io::Error> for FixtureError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}

/// The result type returned by fixture operations.
pub type Result<T> = std::result::Result<T, FixtureError>;
