use std::collections::BTreeMap;

use papo_proto::escape_tag_value;

use crate::Secret;

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

/// Attributes used to add or change an upstream bouncer network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkConfig {
    host: String,
    name: Option<String>,
    port: Option<u16>,
    tls: Option<bool>,
    nickname: Option<String>,
    username: Option<String>,
    realname: Option<String>,
    password: Option<Secret>,
}

impl NetworkConfig {
    /// Creates a configuration for an upstream hostname or IP address.
    #[must_use]
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            name: None,
            port: None,
            tls: None,
            nickname: None,
            username: None,
            realname: None,
            password: None,
        }
    }

    /// Sets the human-readable network name.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the upstream TCP port.
    #[must_use]
    pub const fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Selects whether the upstream connection uses TLS.
    #[must_use]
    pub const fn tls(mut self, tls: bool) -> Self {
        self.tls = Some(tls);
        self
    }

    /// Sets the registration nickname.
    #[must_use]
    pub fn nickname(mut self, nickname: impl Into<String>) -> Self {
        self.nickname = Some(nickname.into());
        self
    }

    /// Sets the registration username.
    #[must_use]
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Sets the registration real name.
    #[must_use]
    pub fn realname(mut self, realname: impl Into<String>) -> Self {
        self.realname = Some(realname.into());
        self
    }

    /// Sets the upstream server password.
    #[must_use]
    pub fn password(mut self, password: impl Into<Secret>) -> Self {
        self.password = Some(password.into());
        self
    }

    #[allow(dead_code)]
    pub(crate) fn attributes(&self) -> String {
        let mut attributes = vec![attribute("host", &self.host)];
        if let Some(value) = &self.name {
            attributes.push(attribute("name", value));
        }
        if let Some(value) = self.port {
            attributes.push(format!("port={value}"));
        }
        if let Some(value) = self.tls {
            attributes.push(format!("tls={}", u8::from(value)));
        }
        for (key, value) in [
            ("nickname", self.nickname.as_deref()),
            ("username", self.username.as_deref()),
            ("realname", self.realname.as_deref()),
            ("pass", self.password.as_ref().map(Secret::expose)),
        ] {
            if let Some(value) = value {
                attributes.push(attribute(key, value));
            }
        }
        attributes.join(";")
    }
}

#[allow(dead_code)]
fn attribute(key: &str, value: &str) -> String {
    format!("{key}={}", escape_tag_value(value))
}

#[cfg(test)]
mod tests {
    use super::NetworkConfig;

    #[test]
    fn network_config_escapes_attributes_and_redacts_password() {
        let config = NetworkConfig::new("irc.example.org")
            .name("Example Network")
            .port(6697)
            .tls(true)
            .realname("Alice; Example")
            .password("unique-bouncer-password-9241");
        assert_eq!(
            config.attributes(),
            r"host=irc.example.org;name=Example\sNetwork;port=6697;tls=1;realname=Alice\:\sExample;pass=unique-bouncer-password-9241"
        );
        assert!(!format!("{config:?}").contains("unique-bouncer-password-9241"));
    }
}
