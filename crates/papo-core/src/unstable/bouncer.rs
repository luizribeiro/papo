use std::{borrow::Borrow, collections::BTreeMap, ops::Deref};

use papo_proto::escape_tag_value;
use papo_proto::{Casemapping, Line, LineRef, Nick};
use web_time::Instant;

use crate::{Feature, RequestError, RequestId, Secret, Session};

/// Stable identifier assigned to a network by a bouncer.
///
/// Equality, ordering, and hashing use the exact received bytes.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NetworkId(String);

impl NetworkId {
    /// Returns the owned identifier text.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl Deref for NetworkId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for NetworkId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for NetworkId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for NetworkId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for NetworkId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl From<&str> for NetworkId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for NetworkId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&String> for NetworkId {
    fn from(value: &String) -> Self {
        Self(value.clone())
    }
}

impl From<&Self> for NetworkId {
    fn from(value: &Self) -> Self {
        value.clone()
    }
}

impl std::fmt::Display for NetworkId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Current connection state of an upstream bouncer network.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
    /// Connection nickname.
    pub nickname: Option<Nick>,
    /// Connection username.
    pub username: Option<String>,
    /// Connection real name.
    pub realname: Option<String>,
    /// Most recent connection error.
    pub error: Option<String>,
    /// Attributes unknown to this version of papo.
    pub other: BTreeMap<String, String>,
}

#[derive(Debug)]
pub(crate) struct NetworkUpdate {
    pub(crate) id: NetworkId,
    pub(crate) removed: bool,
    pub(crate) attributes: BTreeMap<String, Option<String>>,
}

impl NetworkUpdate {
    pub(crate) fn parse(message: &LineRef<'_>) -> Option<Self> {
        if !message.command_is("BOUNCER") || !message.param(0)?.eq_ignore_ascii_case("NETWORK") {
            return None;
        }
        let id = NetworkId::from(message.param(1)?);
        let raw = message.param(2)?;
        if raw == "*" {
            return Some(Self {
                id,
                removed: true,
                attributes: BTreeMap::new(),
            });
        }
        let tagged = format!("@{raw} ATTRS");
        let parsed = LineRef::parse(&tagged).ok()?;
        Some(Self {
            id,
            removed: false,
            attributes: parsed
                .tags()
                .map(|(key, value)| (key.to_owned(), value.map(Into::into)))
                .collect(),
        })
    }
}

impl BouncerNetwork {
    pub(crate) fn empty(id: NetworkId) -> Self {
        Self {
            id,
            name: None,
            state: None,
            host: None,
            port: None,
            tls: None,
            nickname: None,
            username: None,
            realname: None,
            error: None,
            other: BTreeMap::new(),
        }
    }

    pub(crate) fn apply(
        &mut self,
        attributes: BTreeMap<String, Option<String>>,
        casemapping: Casemapping,
    ) {
        for (key, value) in attributes {
            match key.as_str() {
                "name" => self.name = value,
                "state" => self.state = value.as_deref().and_then(parse_state),
                "host" => self.host = value,
                "port" => self.port = value.and_then(|value| value.parse().ok()),
                "tls" => {
                    self.tls = value.as_deref().and_then(|value| match value {
                        "0" => Some(false),
                        "1" => Some(true),
                        _ => None,
                    });
                }
                "nickname" => self.nickname = value.map(|value| casemapping.nick(value)),
                "username" => self.username = value,
                "realname" => self.realname = value,
                "error" => self.error = value,
                _ => match value {
                    Some(value) => _ = self.other.insert(key, value),
                    None => _ = self.other.remove(&key),
                },
            }
        }
    }
}

fn parse_state(value: &str) -> Option<NetworkState> {
    match value {
        "connected" => Some(NetworkState::Connected),
        "connecting" => Some(NetworkState::Connecting),
        "disconnected" => Some(NetworkState::Disconnected),
        _ => None,
    }
}

/// Attributes used to add or change an upstream bouncer network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkConfig {
    host: String,
    name: Option<String>,
    port: Option<u16>,
    tls: Option<bool>,
    nickname: Option<Nick>,
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

    /// Sets the connection nickname.
    #[must_use]
    pub fn nickname(mut self, nickname: impl Into<Nick>) -> Self {
        self.nickname = Some(nickname.into());
        self
    }

    /// Sets the connection username.
    #[must_use]
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Sets the connection real name.
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

    pub(crate) fn attributes(&self) -> Result<String, RequestError> {
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
        let password = self
            .password
            .as_ref()
            .map(|secret| secret.expose().ok_or(RequestError::InvalidSecret))
            .transpose()?;
        for (key, value) in [
            ("nickname", self.nickname.as_deref()),
            ("username", self.username.as_deref()),
            ("realname", self.realname.as_deref()),
            ("pass", password),
        ] {
            if let Some(value) = value {
                attributes.push(attribute(key, value));
            }
        }
        Ok(attributes.join(";"))
    }
}

fn attribute(key: &str, value: &str) -> String {
    format!("{key}={}", escape_tag_value(value))
}

impl Session {
    /// Requests the complete list of upstream bouncer networks.
    ///
    /// # Errors
    ///
    /// Returns an error documented by [`Session::request`].
    pub fn bouncer_networks(&mut self, now: Instant) -> Result<RequestId, RequestError> {
        self.request_for_feature(
            now,
            Line::builder("BOUNCER").param("LISTNETWORKS"),
            Feature::Bouncer,
        )
    }

    /// Adds an upstream bouncer network.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::InvalidSecret`] when the configured password contains bytes
    /// rather than text. Other failures are documented by [`Session::request`].
    pub fn bouncer_add(
        &mut self,
        now: Instant,
        config: &NetworkConfig,
    ) -> Result<RequestId, RequestError> {
        self.request_for_feature(
            now,
            Line::builder("BOUNCER")
                .param("ADDNETWORK")
                .param(&config.attributes()?),
            Feature::Bouncer,
        )
    }

    /// Changes an upstream bouncer network.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::InvalidSecret`] when the configured password contains bytes
    /// rather than text. Other failures are documented by [`Session::request`].
    pub fn bouncer_change(
        &mut self,
        now: Instant,
        id: &NetworkId,
        config: &NetworkConfig,
    ) -> Result<RequestId, RequestError> {
        self.request_for_feature(
            now,
            Line::builder("BOUNCER")
                .param("CHANGENETWORK")
                .param(id.as_ref())
                .param(&config.attributes()?),
            Feature::Bouncer,
        )
    }

    /// Removes an upstream bouncer network.
    ///
    /// # Errors
    ///
    /// Returns an error documented by [`Session::request`].
    pub fn bouncer_remove(
        &mut self,
        now: Instant,
        id: &NetworkId,
    ) -> Result<RequestId, RequestError> {
        self.request_for_feature(
            now,
            Line::builder("BOUNCER")
                .param("DELNETWORK")
                .param(id.as_ref()),
            Feature::Bouncer,
        )
    }
}

#[cfg(test)]
mod tests {
    use papo_proto::{Casemapping, LineRef};

    use super::{BouncerNetwork, NetworkConfig, NetworkState, NetworkUpdate};

    #[test]
    fn network_config_escapes_attributes_and_redacts_password() {
        let config = NetworkConfig::new("irc.example.org")
            .name("Example Network")
            .port(6697)
            .tls(true)
            .realname("Alice; Example")
            .password("unique-bouncer-password-9241");
        assert_eq!(
            config.attributes().unwrap(),
            r"host=irc.example.org;name=Example\sNetwork;port=6697;tls=1;realname=Alice\:\sExample;pass=unique-bouncer-password-9241"
        );
        assert!(!format!("{config:?}").contains("unique-bouncer-password-9241"));
    }

    #[test]
    fn network_config_rejects_a_byte_password() {
        let config =
            NetworkConfig::new("irc.example.org").password(crate::Secret::from_bytes([0xff]));
        assert_eq!(config.attributes(), Err(crate::RequestError::InvalidSecret));
    }

    #[test]
    fn network_updates_preserve_unknown_attributes_and_removals() {
        let message = LineRef::parse(
            r"BOUNCER NETWORK 42 name=My\sNetwork;state=connected;port=6697;tls=1;vendor.example/color=blue",
        )
        .unwrap();
        let update = NetworkUpdate::parse(&message).unwrap();
        let mut network = BouncerNetwork::empty(update.id);
        network.apply(update.attributes, Casemapping::default());
        assert_eq!(network.name.as_deref(), Some("My Network"));
        assert_eq!(network.state, Some(NetworkState::Connected));
        assert_eq!((network.port, network.tls), (Some(6697), Some(true)));
        assert_eq!(network.other["vendor.example/color"], "blue");

        let removal = LineRef::parse("BOUNCER NETWORK 42 name;vendor.example/color").unwrap();
        network.apply(
            NetworkUpdate::parse(&removal).unwrap().attributes,
            Casemapping::default(),
        );
        assert_eq!(network.name, None);
        assert!(network.other.is_empty());
    }

    #[test]
    fn network_deletion_uses_the_star_marker() {
        let message = LineRef::parse("BOUNCER NETWORK 42 *").unwrap();
        assert!(NetworkUpdate::parse(&message).unwrap().removed);
    }
}
