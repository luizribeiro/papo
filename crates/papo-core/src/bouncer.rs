use std::collections::BTreeMap;

/// Stable identifier assigned to a network by a bouncer.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NetworkId(String);

impl NetworkId {
    /// Creates an identifier from its wire representation.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the wire representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NetworkId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Current connection state of an upstream bouncer network.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NetworkState {
    /// Connected to the upstream network.
    Connected,
    /// Attempting to connect to the upstream network.
    Connecting,
    /// Not connected to the upstream network.
    Disconnected,
}

/// One upstream network managed by a bouncer.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BouncerNetwork {
    /// Bouncer-assigned network identifier.
    pub id: NetworkId,
    /// Human-readable network name.
    pub name: Option<String>,
    /// Current upstream connection state.
    pub state: Option<NetworkState>,
    /// Upstream hostname or IP address.
    pub host: Option<String>,
    /// Upstream TCP port.
    pub port: Option<u16>,
    /// Whether the upstream connection uses TLS.
    pub tls: Option<bool>,
    /// Registration nickname.
    pub nickname: Option<String>,
    /// Registration username.
    pub username: Option<String>,
    /// Registration real name.
    pub realname: Option<String>,
    /// Most recent connection error.
    pub error: Option<String>,
    /// Attributes unknown to this version of papo.
    pub other: BTreeMap<String, String>,
}
