use std::{
    borrow::Borrow,
    collections::BTreeMap,
    fmt::{self, Write as _},
    ops::Deref,
};

use papo_proto::{Line, LineRef, Param};
use web_time::Instant;
use zeroize::{Zeroize, Zeroizing};

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

#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn __bouncer_attributes(
    host: Option<&str>,
    name: Option<&str>,
    port: Option<u16>,
    tls: Option<bool>,
    nick: Option<&str>,
    username: Option<&str>,
    realname: Option<&str>,
    password: Option<&Secret>,
) -> Result<Param, RequestError> {
    let password = password.map(Secret::expose);
    let mut attributes = Zeroizing::new(String::new());
    if let Some(value) = host {
        push_attribute(&mut attributes, "host", value);
    }
    if let Some(value) = name {
        push_attribute(&mut attributes, "name", value);
    }
    if let Some(value) = port {
        write!(attributes, ";port={value}").expect("writing to a String cannot fail");
    }
    if let Some(value) = tls {
        write!(attributes, ";tls={}", u8::from(value)).expect("writing to a String cannot fail");
    }
    for (key, value) in [
        ("nickname", nick),
        ("username", username),
        ("realname", realname),
    ] {
        if let Some(value) = value {
            push_attribute(&mut attributes, key, value);
        }
    }
    if let Some(password) = password {
        push_attribute(&mut attributes, "pass", password);
        Ok(Param::from(Secret::new(attributes.as_str())))
    } else {
        Ok(Param::from(attributes.as_str()))
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
    /// Failures are documented by [`Session::request`].
    pub fn bouncer_add(
        &mut self,
        now: Instant,
        attributes: Param,
    ) -> Result<RequestId, RequestError> {
        if attributes.__is_secret() {
            self.require_secure_credentials()?;
        }
        self.request_for_feature(
            now,
            Line::new("BOUNCER").param("ADDNETWORK").param(attributes),
            Feature::Bouncer,
        )
    }

    /// Changes an upstream bouncer network.
    ///
    /// # Errors
    ///
    /// Failures are documented by [`Session::request`].
    pub fn bouncer_change(
        &mut self,
        now: Instant,
        id: &NetworkId,
        attributes: Param,
    ) -> Result<RequestId, RequestError> {
        if attributes.__is_secret() {
            self.require_secure_credentials()?;
        }
        self.request_for_feature(
            now,
            Line::new("BOUNCER")
                .param("CHANGENETWORK")
                .param(id.as_ref())
                .param(attributes),
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

    use super::{BouncerNetwork, NetworkState, NetworkUpdate};

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
    fn bouncer_attributes_escape_values_and_redact_password() {
        let password = crate::Secret::new("unique-bouncer-password-9241");
        let attributes = super::__bouncer_attributes(
            Some("irc.example.org"),
            Some("Example Network"),
            Some(6697),
            Some(true),
            Some("alice"),
            Some("alice"),
            Some("Alice; Example"),
            Some(&password),
        )
        .unwrap();
        assert_eq!(
            &*attributes,
            r"host=irc.example.org;name=Example\sNetwork;port=6697;tls=1;nickname=alice;username=alice;realname=Alice\:\sExample;pass=unique-bouncer-password-9241"
        );
        assert!(!format!("{attributes:?}").contains("unique-bouncer-password-9241"));
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
