use std::fmt;

use crate::{MetadataKey, Nick, Subject, Visibility};
use papo_core::{BatchAbandonReason, StateLimit};

/// An upstream bouncer network was added, updated, or removed.
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BouncerNetworkChanged {
    /// Bouncer-assigned network identifier.
    pub id: crate::NetworkId,
    /// Complete cached network state, or `None` when removed.
    pub network: Option<crate::BouncerNetwork>,
}

/// A user or channel's cached metadata changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MetadataChanged {
    /// User or channel whose metadata changed.
    pub subject: Subject,
    /// Metadata key.
    pub key: MetadataKey,
    /// New server-returned value, or `None` when cleared.
    pub value: Option<String>,
    /// Visibility policy reported by the server.
    pub visibility: Visibility,
}

/// A recoverable protocol or configuration warning.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Warning {
    /// A server batch exceeded a retention bound or deadline.
    #[non_exhaustive]
    BatchAbandoned {
        /// The opaque batch reference.
        id: String,
        /// The batch type from its opening line.
        batch_type: String,
        /// The bound that was reached.
        reason: BatchAbandonReason,
    },
    /// A line from the server could not be decoded or parsed.
    #[non_exhaustive]
    Malformed {
        /// The original bytes, excluding the line ending.
        line: Vec<u8>,
    },
    /// Configured authentication failed and connecting is continuing.
    #[non_exhaustive]
    AuthenticationFailed {
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
    },
    /// Server input exceeded a configured state-retention bound.
    #[non_exhaustive]
    StateLimitReached {
        /// The state collection that rejected another entry.
        kind: StateLimit,
    },
    /// The server rejected nicknames because its monitor list was full.
    #[non_exhaustive]
    MonitorListFull {
        /// The server's advertised monitor-list limit.
        maximum: usize,
        /// Nicknames the server did not add.
        nicks: Vec<Nick>,
    },
    /// The server does not advertise `MONITOR` support.
    MonitorUnsupported,
}

impl fmt::Display for Warning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BatchAbandoned {
                batch_type, reason, ..
            } => {
                write!(formatter, "{batch_type} batch was abandoned: {reason:?}")
            }
            Self::Malformed { line } => {
                write!(
                    formatter,
                    "malformed server line: {}",
                    String::from_utf8_lossy(line)
                )
            }
            Self::AuthenticationFailed { reason } => formatter.write_str(reason),
            Self::StateLimitReached { kind } => {
                write!(formatter, "state limit reached: {kind:?}")
            }
            Self::MonitorListFull { maximum, .. } => {
                write!(
                    formatter,
                    "server monitor list is limited to {maximum} nicknames"
                )
            }
            Self::MonitorUnsupported => formatter.write_str("server does not support MONITOR"),
        }
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use super::Warning;
    use papo_core::{BatchAbandonReason, StateLimit};

    #[test]
    fn warnings_have_human_readable_diagnostics() {
        let warnings = [
            Warning::BatchAbandoned {
                id: "history42".to_owned(),
                batch_type: "chathistory".to_owned(),
                reason: BatchAbandonReason::Timeout,
            },
            Warning::Malformed {
                line: b":ergo.test NOTICE * :caf\xe9".to_vec(),
            },
            Warning::AuthenticationFailed {
                reason: "SASL mechanism unavailable".to_owned(),
            },
            Warning::StateLimitReached {
                kind: StateLimit::Members,
            },
            Warning::MonitorListFull {
                maximum: 100,
                nicks: vec![crate::Casemapping::default().nick("alice")],
            },
            Warning::MonitorUnsupported,
        ];

        for warning in warnings {
            assert!(!warning.to_string().is_empty());
        }
    }
}
