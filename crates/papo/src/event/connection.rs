use crate::{Capabilities, Casemapping, Feature, Nick};

pub use crate::Disconnected;

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

/// An IRC connection ready for use.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Connected {
    /// The nickname assigned by the server.
    pub nick: Nick,
    /// The human-readable welcome text from the server.
    pub welcome: String,
    /// The capabilities enabled on the connection.
    pub caps: Capabilities,
    /// Whether this connection replaced a previous connection.
    pub reconnected: bool,
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

/// Optional behaviors available on an established connection changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct FeaturesChanged {
    /// Behaviors that became available.
    pub added: Vec<Feature>,
    /// Behaviors that became unavailable.
    pub removed: Vec<Feature>,
}

/// Server-defined name rules changed between connections.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CasemappingChanged {
    /// Rules retained from the previous connection or offline seed.
    pub old: Casemapping,
    /// Rules advertised by the current connection.
    pub new: Casemapping,
}
