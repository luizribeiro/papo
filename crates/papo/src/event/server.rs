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
