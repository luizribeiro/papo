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

#[derive(Debug)]
struct Skipped(String);

impl fmt::Display for Skipped {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Skipped {}

/// An error returned by a fixture or scenario.
pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

/// The result type returned by fixtures and scenarios.
pub type Result<T> = std::result::Result<T, Error>;

/// Creates the internal error used to report a runtime skip.
#[doc(hidden)]
pub fn skipped(reason: impl Into<String>) -> Error {
    Box::new(Skipped(reason.into()))
}

/// Returns the reason when an error represents a runtime skip.
#[doc(hidden)]
#[must_use]
pub fn skip_reason(error: &Error) -> Option<&str> {
    error.downcast_ref::<Skipped>().map(|skip| skip.0.as_str())
}

#[cfg(test)]
mod tests {
    use super::{Result, skip_reason, skipped};

    fn arbitrary_error() -> Result<()> {
        Err(std::io::Error::other("fixture failed"))?
    }

    #[test]
    fn result_boxes_errors_and_distinguishes_skips() {
        assert_eq!(arbitrary_error().unwrap_err().to_string(), "fixture failed");
        assert_eq!(
            skip_reason(&skipped("server setup unavailable")),
            Some("server setup unavailable")
        );
    }
}
