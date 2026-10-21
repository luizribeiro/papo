use crate::{Capabilities, Casemapping, Feature, Nick};

pub use crate::Disconnected;

/// A phase of establishing an IRC connection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ConnectPhase {
    /// Opening the underlying transport.
    Transport,
    /// Negotiating transport security.
    Tls,
    /// Discovering and enabling IRC capabilities.
    Capabilities,
    /// Authenticating the connection.
    Authenticating,
    /// Waiting for the server's welcome reply.
    WaitingForWelcome,
}

impl ConnectPhase {
    pub(crate) fn from_core(phase: papo_core::ConnectPhase) -> Option<Self> {
        Some(match phase {
            papo_core::ConnectPhase::Transport => Self::Transport,
            papo_core::ConnectPhase::Tls => Self::Tls,
            papo_core::ConnectPhase::Capabilities => Self::Capabilities,
            papo_core::ConnectPhase::Authenticating => Self::Authenticating,
            papo_core::ConnectPhase::WaitingForWelcome => Self::WaitingForWelcome,
            _ => return None,
        })
    }
}

/// Progress while establishing an IRC connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Connecting {
    /// One-based attempt number since the last stable connection.
    pub attempt: usize,
    /// Work currently in progress.
    pub phase: ConnectPhase,
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

/// A complete message of the day sent while connecting.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Motd {
    /// Text from each `RPL_MOTD` numeric, in wire order.
    ///
    /// This event is absent when the server reports `ERR_NOMOTD`.
    pub lines: Vec<String>,
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
