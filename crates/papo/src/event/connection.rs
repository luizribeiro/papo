use crate::{Capabilities, Feature, Nick};

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
