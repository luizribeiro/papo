use std::fmt;

use crate::{Capabilities, Nick};

/// A connection completed IRC registration.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Registered {
    /// The nickname assigned by the server.
    pub nick: Nick,
    /// The human-readable welcome text from the server.
    pub text: String,
    /// The capabilities enabled on the connection.
    pub caps: Capabilities,
    /// Whether this registration replaced a previous connection.
    pub reconnected: bool,
}

/// A connection ended.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Disconnected {
    /// A human-readable explanation suitable for diagnostics.
    pub reason: String,
    /// Whether the client will try to connect again.
    pub will_retry: bool,
}

impl Disconnected {
    /// Whether another connection attempt is scheduled.
    #[must_use]
    pub const fn is_retrying(&self) -> bool {
        self.will_retry
    }
}

impl fmt::Display for Disconnected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

/// The capabilities enabled on an established connection changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CapsChanged {
    /// Newly enabled capabilities and updated advertised values.
    pub added: Capabilities,
    /// Capabilities removed by the server.
    pub removed: Capabilities,
}
