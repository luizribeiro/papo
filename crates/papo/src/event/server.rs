use std::fmt;

/// A target's cached metadata changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MetadataChanged {
    /// Nickname or channel reported by the server.
    pub target: String,
    /// Metadata key.
    pub key: String,
    /// New server-returned value, or `None` when cleared.
    pub value: Option<String>,
    /// Visibility policy reported by the server.
    pub visibility: String,
}

/// An account's configured persistence preference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PersistenceSetting {
    /// Keep the account's client active while disconnected.
    On,
    /// Disconnect the account's client normally.
    Off,
    /// Use the server's default persistence policy.
    Default,
}

/// The server reported the account's persistence state.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PersistenceChanged {
    /// The configured persistence preference.
    pub setting: PersistenceSetting,
    /// Whether persistence is currently effective.
    pub effective: bool,
}

/// Why an incomplete server batch was released as individual messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BatchAbandonReason {
    /// The server opened more batches than configured.
    OpenBatchLimit,
    /// A batch exceeded its configured retained-line limit.
    LineLimit,
    /// A multiline batch exceeded its negotiated text-byte limit.
    ByteLimit,
    /// A batch did not close before its deadline.
    Timeout,
}

/// A bounded server-controlled state collection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StateLimit {
    /// Joined channels.
    Channels,
    /// Members retained for one channel.
    Members,
    /// Targets retained in the metadata cache.
    MetadataTargets,
    /// Metadata keys retained for one target.
    MetadataKeys,
}

/// A recoverable protocol or configuration warning.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Warning {
    /// A server batch exceeded a retention bound or deadline.
    #[non_exhaustive]
    BatchAbandoned {
        /// The opaque batch reference.
        reference: String,
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
    /// Configured authentication failed and registration is continuing.
    #[non_exhaustive]
    AuthenticationFailed {
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
    },
    /// Server input exceeded a configured state-retention bound.
    #[non_exhaustive]
    StateLimitReached {
        /// The state collection that rejected another entry.
        limit: StateLimit,
    },
    /// The server rejected nicknames because its monitor list was full.
    #[non_exhaustive]
    MonitorListFull {
        /// The server's advertised monitor-list limit.
        limit: usize,
        /// Nicknames the server did not add.
        nicks: Vec<String>,
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
            Self::StateLimitReached { limit } => {
                write!(formatter, "state limit reached: {limit:?}")
            }
            Self::MonitorListFull { limit, .. } => {
                write!(
                    formatter,
                    "server monitor list is limited to {limit} nicknames"
                )
            }
            Self::MonitorUnsupported => formatter.write_str("server does not support MONITOR"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BatchAbandonReason, StateLimit, Warning};

    #[test]
    fn warnings_have_human_readable_diagnostics() {
        let warnings = [
            Warning::BatchAbandoned {
                reference: "history42".to_owned(),
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
                limit: StateLimit::Members,
            },
            Warning::MonitorListFull {
                limit: 100,
                nicks: vec!["alice".to_owned()],
            },
            Warning::MonitorUnsupported,
        ];

        for warning in warnings {
            assert!(!warning.to_string().is_empty());
        }
    }
}
