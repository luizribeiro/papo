use std::{
    borrow::Borrow,
    collections::BTreeMap,
    fmt::{self, Write as _},
    ops::Deref,
};

use papo_proto::{Line, LineRef, Param};
use web_time::Instant;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    Feature, RequestError, RequestId, Secret, Session,
    server_url::{InvalidServerUrl, ServerUrl, ServerUrlScheme},
};

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
///
/// Display uses lowercase protocol spellings so state output matches soju's wire values.
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

impl fmt::Display for NetworkState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Connected => "connected",
            Self::Connecting => "connecting",
            Self::Disconnected => "disconnected",
        })
    }
}

/// Why an upstream network URL could not be parsed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InvalidNetworkUrl {
    /// The URL is empty.
    Empty,
    /// The URL scheme is missing or unsupported.
    UnsupportedScheme(String),
    /// The hostname or bracketed IPv6 literal is malformed.
    InvalidHost(String),
    /// The port is missing, zero, or greater than 65535.
    InvalidPort(String),
    /// The URL contains a path.
    UnexpectedPath(String),
    /// The shared server URL parser rejected the input.
    Invalid(InvalidServerUrl),
}

impl fmt::Display for InvalidNetworkUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("upstream network URL is empty"),
            Self::UnsupportedScheme(scheme) => write!(formatter, "unsupported scheme {scheme:?}"),
            Self::InvalidHost(host) => write!(formatter, "invalid upstream host {host:?}"),
            Self::InvalidPort(port) => write!(formatter, "invalid service port {port:?}"),
            Self::UnexpectedPath(path) => write!(formatter, "unexpected URL path {path:?}"),
            Self::Invalid(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for InvalidNetworkUrl {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invalid(error) => Some(error),
            _ => None,
        }
    }
}

impl From<InvalidServerUrl> for InvalidNetworkUrl {
    fn from(error: InvalidServerUrl) -> Self {
        Self::Invalid(error)
    }
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
    pub nick: Option<String>,
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
        let tagged = Zeroizing::new(format!("@{raw} ATTRS"));
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
            nick: None,
            username: None,
            realname: None,
            error: None,
            other: BTreeMap::new(),
        }
    }

    pub(crate) fn apply(&mut self, attributes: BTreeMap<String, Option<String>>) {
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
                "nickname" => self.nick = value,
                "username" => self.username = value,
                "realname" => self.realname = value,
                "error" => self.error = value,
                "pass" => {
                    if let Some(mut value) = value {
                        value.zeroize();
                    }
                }
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
    nick: Option<String>,
    username: Option<String>,
    realname: Option<String>,
    password: Option<Secret>,
}

impl NetworkConfig {
    /// Creates a configuration from an `irc://` or `ircs://` URL.
    ///
    /// Brackets are required around IPv6 literals. When the URL omits a port, IRC defaults to
    /// 6667 and IRC over TLS defaults to 6697.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidNetworkUrl`] for an unsupported scheme, malformed host or port, or a URL
    /// containing a path, query, fragment, or user information.
    pub fn from_url(url: &str) -> Result<Self, InvalidNetworkUrl> {
        let url = url.parse::<ServerUrl>().map_err(InvalidNetworkUrl::from)?;
        let tls = match url.scheme() {
            ServerUrlScheme::Irc => false,
            ServerUrlScheme::Ircs => true,
            scheme => {
                return Err(InvalidServerUrl::UnsupportedScheme(scheme.to_string()).into());
            }
        };
        Ok(Self::new(url.host()).port(url.port()).tls(tls))
    }

    /// Creates a configuration for an upstream hostname or IP address.
    #[must_use]
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            name: None,
            port: None,
            tls: None,
            nick: None,
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
    pub fn nick(mut self, nick: impl AsRef<str>) -> Self {
        self.nick = Some(nick.as_ref().to_owned());
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

    pub(crate) fn attributes(&self) -> Result<Param, RequestError> {
        let password = self
            .password
            .as_ref()
            .map(|secret| secret.expose().ok_or(RequestError::InvalidSecret))
            .transpose()?;
        let mut attributes = Zeroizing::new(String::new());
        push_attribute(&mut attributes, "host", &self.host);
        if let Some(value) = &self.name {
            push_attribute(&mut attributes, "name", value);
        }
        if let Some(value) = self.port {
            write!(attributes, ";port={value}").expect("writing to a String cannot fail");
        }
        if let Some(value) = self.tls {
            write!(attributes, ";tls={}", u8::from(value))
                .expect("writing to a String cannot fail");
        }
        for (key, value) in [
            ("nickname", self.nick.as_deref()),
            ("username", self.username.as_deref()),
            ("realname", self.realname.as_deref()),
        ] {
            if let Some(value) = value {
                push_attribute(&mut attributes, key, value);
            }
        }
        if let Some(password) = password {
            push_attribute(&mut attributes, "pass", password);
            Ok(Param::from(Secret::from(attributes.as_str())))
        } else {
            Ok(Param::from(attributes.as_str()))
        }
    }
}

fn push_attribute(attributes: &mut String, key: &str, value: &str) {
    if !attributes.is_empty() {
        attributes.push(';');
    }
    attributes.push_str(key);
    attributes.push('=');
    for character in value.chars() {
        match character {
            ';' => attributes.push_str(r"\:"),
            ' ' => attributes.push_str(r"\s"),
            '\\' => attributes.push_str(r"\\"),
            '\r' => attributes.push_str(r"\r"),
            '\n' => attributes.push_str(r"\n"),
            other => attributes.push(other),
        }
    }
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
            Line::new("BOUNCER").param("LISTNETWORKS"),
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
            Line::new("BOUNCER")
                .param("ADDNETWORK")
                .param(config.attributes()?),
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
            Line::new("BOUNCER")
                .param("CHANGENETWORK")
                .param(id.as_ref())
                .param(config.attributes()?),
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
            Line::new("BOUNCER").param("DELNETWORK").param(id.as_ref()),
            Feature::Bouncer,
        )
    }
}

#[cfg(test)]
mod tests {
    use papo_proto::LineRef;

    use super::{BouncerNetwork, InvalidNetworkUrl, NetworkConfig, NetworkState, NetworkUpdate};
    use crate::server_url::InvalidServerUrl;

    #[test]
    fn network_states_display_as_protocol_words() {
        for (state, expected) in [
            (NetworkState::Connected, "connected"),
            (NetworkState::Connecting, "connecting"),
            (NetworkState::Disconnected, "disconnected"),
        ] {
            assert_eq!(state.to_string(), expected);
        }
    }

    #[test]
    fn network_config_parses_irc_urls() {
        let config = NetworkConfig::from_url("ircs://irc.oftc.net").unwrap();
        assert_eq!(
            &*config.attributes().unwrap(),
            "host=irc.oftc.net;port=6697;tls=1"
        );
    }

    #[test]
    fn network_config_parses_plaintext_and_ipv6_urls() {
        for (url, expected) in [
            (
                "irc://irc.oftc.net:7000",
                "host=irc.oftc.net;port=7000;tls=0",
            ),
            (
                "irc://[2001:db8::7]:6669",
                "host=2001:db8::7;port=6669;tls=0",
            ),
        ] {
            assert_eq!(
                &*NetworkConfig::from_url(url).unwrap().attributes().unwrap(),
                expected
            );
        }
    }

    #[test]
    fn network_config_rejects_malformed_urls() {
        for url in [
            "",
            "irc.oftc.net:6697",
            "wss://irc.oftc.net",
            "irc://",
            "irc://[2001:db8::7",
            "irc://2001:db8::7",
            "irc://irc.oftc.net:0",
            "irc://irc.oftc.net:65536",
            "irc://irc.oftc.net:",
            "irc://alice@irc.oftc.net",
            "irc://irc.oftc.net/network",
            "irc://irc.oftc.net/",
            "irc://irc.oftc.net?network=oftc",
            "irc://irc.oftc.net#network",
        ] {
            assert!(NetworkConfig::from_url(url).is_err(), "accepted {url:?}");
        }
        assert_eq!(
            NetworkConfig::from_url("irc://irc.oftc.net:0"),
            Err(InvalidNetworkUrl::Invalid(InvalidServerUrl::InvalidPort(
                "0".into()
            )))
        );
        assert_eq!(
            NetworkConfig::from_url("wss://irc.oftc.net"),
            Err(InvalidNetworkUrl::Invalid(
                InvalidServerUrl::UnsupportedScheme("wss".into())
            ))
        );
        assert_eq!(
            NetworkConfig::from_url("irc://[2001:db8::7]6697"),
            Err(InvalidNetworkUrl::Invalid(InvalidServerUrl::InvalidHost(
                "[2001:db8::7]6697".into()
            )))
        );
    }

    #[test]
    fn invalid_network_urls_display_exact_diagnostics() {
        for (url, expected) in [
            ("", "server URL is empty"),
            ("wss://irc.oftc.net", "unsupported scheme \"wss\""),
            (
                "irc://[2001:db8::7]6697",
                "invalid server host \"[2001:db8::7]6697\"",
            ),
            ("irc://irc.oftc.net:65536", "invalid service port \"65536\""),
            (
                "irc://irc.oftc.net/network",
                "IRC server URL cannot contain path \"/network\"",
            ),
        ] {
            let error: InvalidNetworkUrl = NetworkConfig::from_url(url).unwrap_err();
            assert_eq!(error.to_string(), expected);
            assert!(std::error::Error::source(&error).is_some());
        }
    }

    #[test]
    fn network_config_escapes_attributes_and_redacts_password() {
        let config = NetworkConfig::new("irc.example.org")
            .name("Example Network")
            .port(6697)
            .tls(true)
            .realname("Alice; Example")
            .password("unique-bouncer-password-9241");
        assert_eq!(
            &*config.attributes().unwrap(),
            r"host=irc.example.org;name=Example\sNetwork;port=6697;tls=1;realname=Alice\:\sExample;pass=unique-bouncer-password-9241"
        );
        assert!(!format!("{config:?}").contains("unique-bouncer-password-9241"));
    }

    #[test]
    fn network_config_rejects_a_byte_password() {
        let config =
            NetworkConfig::new("irc.example.org").password(crate::Secret::from_bytes([0xff]));
        assert!(matches!(
            config.attributes(),
            Err(crate::RequestError::InvalidSecret)
        ));
    }

    #[test]
    fn network_updates_preserve_unknown_attributes_and_removals() {
        let message = LineRef::parse(
            r"BOUNCER NETWORK 42 name=My\sNetwork;state=connected;port=6697;tls=1;vendor.example/color=blue",
        )
        .unwrap();
        let update = NetworkUpdate::parse(&message).unwrap();
        let mut network = BouncerNetwork::empty(update.id);
        network.apply(update.attributes);
        assert_eq!(network.name.as_deref(), Some("My Network"));
        assert_eq!(network.state, Some(NetworkState::Connected));
        assert_eq!((network.port, network.tls), (Some(6697), Some(true)));
        assert_eq!(network.other["vendor.example/color"], "blue");

        let removal = LineRef::parse("BOUNCER NETWORK 42 name;vendor.example/color").unwrap();
        network.apply(NetworkUpdate::parse(&removal).unwrap().attributes);
        assert_eq!(network.name, None);
        assert!(network.other.is_empty());
    }

    #[test]
    fn network_updates_discard_password_attributes() {
        let message = LineRef::parse(
            r"BOUNCER NETWORK 42 host=irc.example.org;pass=server-supplied-password;vendor.example/color=blue",
        )
        .unwrap();
        let update = NetworkUpdate::parse(&message).unwrap();
        let mut network = BouncerNetwork::empty(update.id);
        network.apply(update.attributes);
        assert_eq!(network.other.get("vendor.example/color").unwrap(), "blue");
        assert!(!network.other.contains_key("pass"));
        assert!(!format!("{network:?}").contains("server-supplied-password"));
    }

    #[test]
    fn network_deletion_uses_the_star_marker() {
        let message = LineRef::parse("BOUNCER NETWORK 42 *").unwrap();
        assert!(NetworkUpdate::parse(&message).unwrap().removed);
    }
}
