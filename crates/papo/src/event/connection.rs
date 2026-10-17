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

/// The capabilities enabled on an established connection changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CapsChanged {
    /// Newly enabled capabilities and updated advertised values.
    pub added: Capabilities,
    /// Capabilities removed by the server.
    pub removed: Capabilities,
}
