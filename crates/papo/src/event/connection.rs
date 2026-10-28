use std::time::SystemTime;

use crate::{Capabilities, Casemapping, Conversation, Feature, MessageId, Nick};

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

/// A warning that events were discarded because the event buffer was full.
///
/// papo first discards changes already reflected by [`crate::Client::state`], then other
/// non-message events, and finally message-like events. Re-read State to recover the first kind.
/// Use [`crate::Client::history`] with [`Self::gaps`] to recover messages when history is
/// available; papo does not fill gaps automatically.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Lagged {
    /// Number of events discarded since the previous lag warning.
    pub dropped: u64,
    /// Message-like events discarded from each affected conversation.
    ///
    /// Use [`crate::Client::history`] to recover these ranges when history is available. The
    /// list is bounded to 32 conversations, so [`Self::dropped`] can exceed the sum of its
    /// entries.
    pub gaps: Vec<MessageGap>,
}

/// A range of message-like events discarded from one conversation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MessageGap {
    /// Conversation whose history has a gap.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub conversation: Conversation,
    /// First available message identifier among the discarded events.
    pub first_msgid: Option<MessageId>,
    /// Last available message identifier among the discarded events.
    pub last_msgid: Option<MessageId>,
    /// First available server timestamp among the discarded events.
    pub first_server_time: Option<SystemTime>,
    /// Last available server timestamp among the discarded events.
    pub last_server_time: Option<SystemTime>,
    /// Number of message-like events discarded from this conversation.
    pub dropped: u64,
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

/// Server-defined name rules changed.
///
/// Applications with maps or sets keyed by names must rebuild them because existing names retain
/// the old casemapping and deliberately do not compare equal to names built with the new rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CasemappingChanged {
    /// Rules retained from the previous connection or offline seed.
    pub old: Casemapping,
    /// Rules advertised by the current connection.
    pub new: Casemapping,
}
