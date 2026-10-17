use std::str::FromStr;

use thiserror::Error;

const REJECTED_ADDRESS_CHARACTERS: [char; 5] = ['?', '#', '@', ' ', '\t'];

/// An invalid client configuration value.
#[derive(Clone, Debug, Eq, Error, Hash, PartialEq)]
#[non_exhaustive]
pub enum InvalidInput {
    /// The address is empty.
    #[error("server address {0:?} is empty")]
    Empty(String),
    /// The URL scheme is not supported.
    #[error("unsupported scheme {0:?}")]
    UnsupportedScheme(String),
    /// The host or bracketed IPv6 literal is malformed.
    #[error("invalid server host {0:?}")]
    InvalidHost(String),
    /// The service port is missing, zero, or greater than 65535.
    #[error("invalid service port {0:?}")]
    InvalidPort(String),
    /// A TCP IRC URL contains a path, query, or fragment.
    #[error("IRC server address cannot contain path {0:?}")]
    UnexpectedPath(String),
    /// A capability cannot be removed because papo does not request it by default.
    #[error("papo does not request capability {0:?} by default")]
    UnknownCapability(String),
    /// The event queue capacity is zero.
    #[error("event queue capacity {0} must be positive")]
    EventCapacity(usize),
    /// An outbound IRC message cannot be represented on the wire.
    #[error("invalid IRC message: {0}")]
    InvalidMessage(String),
    /// Outgoing chat text has no nonempty lines.
    #[error("chat text is empty")]
    EmptyText,
    /// A history or reply reference lacks a server message identifier.
    #[error("the message reference has no server identifier")]
    MessageReference,
    /// An account operation omitted a required email address.
    #[error("account registration requires an email address")]
    EmailRequired,
    /// The server requires an account name to match the current nickname.
    #[error("the server does not support a custom account name")]
    CustomAccountName,
    /// A metadata key does not follow the negotiated syntax.
    #[error("invalid metadata key")]
    MetadataKey,
    /// A metadata value exceeds the server's advertised byte limit.
    #[error("metadata value length {actual} exceeds limit {maximum}")]
    MetadataValue {
        /// The advertised maximum length.
        maximum: usize,
        /// The supplied length.
        actual: usize,
    },
    /// A metadata subscription exceeds the server's advertised limit.
    #[error("metadata subscription count {actual} exceeds limit {maximum}")]
    MetadataSubscriptions {
        /// The advertised maximum count.
        maximum: usize,
        /// The supplied count.
        actual: usize,
    },
    /// A relayed nickname lacks an advertised separator.
    #[error("relayed nickname has no allowed separator")]
    RelayNickname,
    /// A Web Push endpoint is not a nonempty HTTPS URL.
    #[error("invalid Web Push endpoint")]
    WebPushEndpoint,
    /// A Web Push key is not valid URL-safe base64.
    #[error("invalid Web Push key")]
    WebPushKey,
}

/// The network endpoint used by an IRC client.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Endpoint(Result<ServerAddress, InvalidInput>);

impl Endpoint {
    /// Creates a TLS-protected TCP endpoint.
    ///
    /// ```
    /// let endpoint = papo::Endpoint::tls("irc.example.net", 6697);
    /// # _ = endpoint;
    /// ```
    ///
    #[cfg(any(
        not(target_family = "wasm"),
        all(target_os = "wasi", not(target_env = "p1"))
    ))]
    pub fn tls(host: impl Into<String>, port: u16) -> Self {
        Self::tcp(host, port, Protocol::Ircs)
    }

    /// Creates an unencrypted TCP endpoint that remains eligible for STS upgrades.
    ///
    /// ```
    /// let endpoint = papo::Endpoint::plaintext("irc.example.net", 6667);
    /// # _ = endpoint;
    /// ```
    ///
    #[cfg(any(
        not(target_family = "wasm"),
        all(target_os = "wasi", not(target_env = "p1"))
    ))]
    pub fn plaintext(host: impl Into<String>, port: u16) -> Self {
        Self::tcp(host, port, Protocol::Irc)
    }

    /// Creates a WebSocket endpoint from a `ws://` or `wss://` URL.
    ///
    /// ```
    /// let endpoint = papo::Endpoint::websocket("wss://irc.example.net/webirc");
    /// # _ = endpoint;
    /// ```
    ///
    pub fn websocket(url: impl AsRef<str>) -> Self {
        let url = url.as_ref();
        let Some((scheme, _)) = url.split_once("://") else {
            return Self(Err(InvalidInput::UnsupportedScheme(String::new())));
        };
        if !scheme.eq_ignore_ascii_case("ws") && !scheme.eq_ignore_ascii_case("wss") {
            return Self(Err(InvalidInput::UnsupportedScheme(scheme.to_owned())));
        }
        Self(url.parse())
    }

    #[cfg(any(
        not(target_family = "wasm"),
        all(target_os = "wasi", not(target_env = "p1"))
    ))]
    fn tcp(host: impl Into<String>, port: u16, protocol: Protocol) -> Self {
        let address = ServerAddress {
            protocol,
            host: host.into(),
            port,
            explicit_port: true,
            websocket_path: None,
        };
        if port == 0 {
            Self(Err(InvalidInput::InvalidPort(port.to_string())))
        } else {
            Self(Ok(address))
        }
    }

    pub(crate) fn into_address(self) -> Result<ServerAddress, InvalidInput> {
        self.0
    }
}

mod private {
    use super::Endpoint;

    pub trait Sealed {
        fn into_endpoint(self) -> Endpoint;
    }
}

/// A value accepted by [`crate::Client::builder`].
///
/// This trait is sealed; it is implemented for [`Endpoint`], `&str`, and [`String`].
pub trait IntoEndpoint: private::Sealed {}

impl IntoEndpoint for Endpoint {}
impl IntoEndpoint for &str {}
impl IntoEndpoint for String {}

impl private::Sealed for Endpoint {
    fn into_endpoint(self) -> Endpoint {
        self
    }
}

impl private::Sealed for &str {
    fn into_endpoint(self) -> Endpoint {
        hostname(self.to_owned())
    }
}

impl private::Sealed for String {
    fn into_endpoint(self) -> Endpoint {
        hostname(self)
    }
}

pub(crate) fn convert(value: impl IntoEndpoint) -> Endpoint {
    private::Sealed::into_endpoint(value)
}

fn hostname(host: String) -> Endpoint {
    if host.is_empty()
        || host.contains("://")
        || host.contains([':', '/', '@', '?', '#'])
        || host.chars().any(char::is_whitespace)
    {
        return Endpoint(Err(InvalidInput::InvalidHost(host)));
    }
    Endpoint(Ok(ServerAddress {
        protocol: Protocol::Ircs,
        host,
        port: 6697,
        explicit_port: false,
        websocket_path: None,
    }))
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Protocol {
    Irc,
    Ircs,
    Ws,
    Wss,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ServerAddress {
    pub(crate) protocol: Protocol,
    pub(crate) host: String,
    pub(crate) port: u16,
    explicit_port: bool,
    websocket_path: Option<String>,
}

impl FromStr for ServerAddress {
    type Err = InvalidInput;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.is_empty() {
            return Err(InvalidInput::Empty(input.to_owned()));
        }
        let (protocol, remainder) = match input.split_once("://") {
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("irc") => (Protocol::Irc, rest),
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("ircs") => (Protocol::Ircs, rest),
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("ws") => (Protocol::Ws, rest),
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("wss") => (Protocol::Wss, rest),
            Some((scheme, _)) => return Err(InvalidInput::UnsupportedScheme(scheme.to_owned())),
            None => (Protocol::Ircs, input),
        };
        let websocket = matches!(protocol, Protocol::Ws | Protocol::Wss);
        let (authority, path) = remainder.split_once('/').unwrap_or((remainder, ""));
        if !websocket && (!path.is_empty() || remainder.ends_with('/')) {
            return Err(InvalidInput::UnexpectedPath(path.to_owned()));
        }
        if authority.contains(REJECTED_ADDRESS_CHARACTERS) {
            return Err(InvalidInput::InvalidHost(authority.to_owned()));
        }
        if path.contains(REJECTED_ADDRESS_CHARACTERS) {
            return Err(InvalidInput::InvalidHost(path.to_owned()));
        }
        let default_port = default_port(protocol);
        let (host, port) = parse_authority(authority, default_port)?;
        Ok(Self {
            protocol,
            host,
            port,
            explicit_port: port != default_port || authority.ends_with(&format!(":{port}")),
            websocket_path: websocket.then(|| format!("/{path}")),
        })
    }
}

impl ServerAddress {
    pub(crate) fn set_port(&mut self, port: u16) {
        assert!(port > 0, "server port must be positive");
        self.port = port;
        self.explicit_port = true;
    }

    pub(crate) fn set_tls(&mut self, enabled: bool) {
        self.protocol = match (self.websocket_path.is_some(), enabled) {
            (false, false) => Protocol::Irc,
            (false, true) => Protocol::Ircs,
            (true, false) => Protocol::Ws,
            (true, true) => Protocol::Wss,
        };
        if !self.explicit_port {
            self.port = default_port(self.protocol);
        }
    }

    pub(crate) fn is_secure(&self) -> bool {
        matches!(self.protocol, Protocol::Ircs | Protocol::Wss)
    }

    #[cfg(sockets)]
    pub(crate) fn websocket_url(&self) -> Option<String> {
        let scheme = match self.protocol {
            Protocol::Ws => "ws",
            Protocol::Wss => "wss",
            Protocol::Irc | Protocol::Ircs => return None,
        };
        let (open, close) = if self.host.contains(':') {
            ("[", "]")
        } else {
            ("", "")
        };
        Some(format!(
            "{scheme}://{open}{}{close}:{}{}",
            self.host,
            self.port,
            self.websocket_path.as_deref().unwrap_or("/")
        ))
    }
}

const fn default_port(protocol: Protocol) -> u16 {
    match protocol {
        Protocol::Irc => 6667,
        Protocol::Ircs => 6697,
        Protocol::Ws => 80,
        Protocol::Wss => 443,
    }
}

fn parse_authority(authority: &str, default_port: u16) -> Result<(String, u16), InvalidInput> {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, suffix) = rest
            .split_once(']')
            .ok_or_else(|| InvalidInput::InvalidHost(authority.to_owned()))?;
        let port = suffix.strip_prefix(':');
        if host.is_empty() || (!suffix.is_empty() && port.is_none()) {
            return Err(InvalidInput::InvalidHost(authority.to_owned()));
        }
        (host, port)
    } else {
        if authority.matches(':').count() > 1 {
            return Err(InvalidInput::InvalidHost(authority.to_owned()));
        }
        authority
            .split_once(':')
            .map_or((authority, None), |(host, port)| (host, Some(port)))
    };
    if host.is_empty() || host.contains(['[', ']', ' ', '\t']) {
        return Err(InvalidInput::InvalidHost(host.to_owned()));
    }
    let port = port.map_or(Ok(default_port), |port| {
        port.parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| InvalidInput::InvalidPort(port.to_owned()))
    })?;
    Ok((host.to_owned(), port))
}

#[cfg(test)]
mod tests {
    use super::{Endpoint, InvalidInput, Protocol, ServerAddress};

    #[test]
    fn typed_constructors_preserve_transport_details() {
        let tls = Endpoint::tls("irc.example.net", 7443).0.unwrap();
        assert_eq!(
            (tls.protocol, tls.host.as_str(), tls.port),
            (Protocol::Ircs, "irc.example.net", 7443)
        );

        let plain = Endpoint::plaintext("irc.example.net", 7000).0.unwrap();
        assert_eq!(
            (plain.protocol, plain.host.as_str(), plain.port),
            (Protocol::Irc, "irc.example.net", 7000)
        );

        let websocket = Endpoint::websocket("wss://irc.example.net:8443/webirc")
            .0
            .unwrap();
        assert_eq!(
            (websocket.protocol, websocket.host.as_str(), websocket.port),
            (Protocol::Wss, "irc.example.net", 8443)
        );
        assert_eq!(websocket.websocket_path.as_deref(), Some("/webirc"));
    }

    #[test]
    fn websocket_rejects_non_websocket_schemes_and_userinfo() {
        for url in [
            "irc.example.net",
            "irc://irc.example.net",
            "https://irc.example.net/webirc",
            "wss://alice@irc.example.net/webirc",
        ] {
            assert!(Endpoint::websocket(url).0.is_err(), "accepted {url:?}");
        }
    }

    #[test]
    fn typed_zero_ports_are_deferred_as_explicit_input_errors() {
        for endpoint in [
            Endpoint::tls("irc.example.net", 0),
            Endpoint::plaintext("irc.example.net", 0),
        ] {
            assert_eq!(
                endpoint.into_address(),
                Err(InvalidInput::InvalidPort("0".into()))
            );
        }
    }

    #[test]
    fn parses_every_server_address_form() {
        let cases = [
            ("irc.example.net", Protocol::Ircs, "irc.example.net", 6697),
            (
                "irc.example.net:7000",
                Protocol::Ircs,
                "irc.example.net",
                7000,
            ),
            (
                "ircs://irc.example.net",
                Protocol::Ircs,
                "irc.example.net",
                6697,
            ),
            (
                "irc://irc.example.net",
                Protocol::Irc,
                "irc.example.net",
                6667,
            ),
            ("[2001:db8::1]:6697", Protocol::Ircs, "2001:db8::1", 6697),
            (
                "ws://irc.example.net/webirc",
                Protocol::Ws,
                "irc.example.net",
                80,
            ),
            (
                "wss://[2001:db8::2]:8443/irc",
                Protocol::Wss,
                "2001:db8::2",
                8443,
            ),
            (
                "IRCS://irc.example.net",
                Protocol::Ircs,
                "irc.example.net",
                6697,
            ),
            (
                "Irc://irc.example.net",
                Protocol::Irc,
                "irc.example.net",
                6667,
            ),
            (
                "WSS://irc.example.net",
                Protocol::Wss,
                "irc.example.net",
                443,
            ),
        ];
        for (input, protocol, host, port) in cases {
            let address: ServerAddress = input.parse().unwrap();
            assert_eq!(
                (address.protocol, address.host.as_str(), address.port),
                (protocol, host, port)
            );
            assert_eq!(
                address.websocket_path.is_some(),
                matches!(protocol, Protocol::Ws | Protocol::Wss)
            );
        }
    }

    #[test]
    fn rejects_malformed_addresses_at_each_boundary() {
        for input in [
            "",
            "irc://",
            "[::1",
            "::1",
            "irc://host/path",
            "host:0",
            "host:65536",
            "host:",
        ] {
            assert!(
                input.parse::<ServerAddress>().is_err(),
                "accepted {input:?}"
            );
        }
        assert_eq!(
            "ftp://irc.example.net"
                .parse::<ServerAddress>()
                .unwrap_err(),
            InvalidInput::UnsupportedScheme("ftp".into())
        );
        assert_eq!("host:1".parse::<ServerAddress>().unwrap().port, 1);
        assert_eq!("host:65535".parse::<ServerAddress>().unwrap().port, 65535);
    }

    #[test]
    fn rejects_url_components_and_ipv6_suffixes_with_typed_errors() {
        for input in [
            "alice@irc.example.net",
            "irc.example.net?x=1",
            "irc.example.net#frag",
            "[::1]x",
        ] {
            assert!(
                matches!(
                    input.parse::<ServerAddress>(),
                    Err(InvalidInput::InvalidHost(offending)) if input.contains(&offending)
                ),
                "accepted {input:?}"
            );
        }
    }

    #[test]
    fn changing_security_updates_each_implicit_port() {
        for (input, enabled, expected_protocol, expected_port) in [
            ("irc://irc.example.net", true, Protocol::Ircs, 6697),
            ("ircs://irc.example.net", false, Protocol::Irc, 6667),
            ("ws://irc.example.net", true, Protocol::Wss, 443),
            ("wss://irc.example.net", false, Protocol::Ws, 80),
        ] {
            let mut address: ServerAddress = input.parse().unwrap();
            address.set_tls(enabled);
            assert_eq!(
                (address.protocol, address.port),
                (expected_protocol, expected_port)
            );
        }
    }

    #[test]
    fn builds_websocket_urls_with_brackets_and_default_paths() {
        let ipv6: ServerAddress = "wss://[2001:db8::2]:8443/irc".parse().unwrap();
        assert_eq!(
            ipv6.websocket_url().as_deref(),
            Some("wss://[2001:db8::2]:8443/irc")
        );

        let hostname: ServerAddress = "ws://irc.example.net".parse().unwrap();
        assert_eq!(
            hostname.websocket_url().as_deref(),
            Some("ws://irc.example.net:80/")
        );
    }

    #[test]
    fn rejects_the_same_characters_in_authorities_and_paths() {
        for character in ['?', '#', '@', ' ', '\t'] {
            for input in [
                format!("ws://irc{character}example.net/irc"),
                format!("ws://irc.example.net/irc{character}gateway"),
            ] {
                assert!(
                    matches!(
                        input.parse::<ServerAddress>(),
                        Err(InvalidInput::InvalidHost(offending)) if input.contains(&offending)
                    ),
                    "accepted {input:?}"
                );
            }
        }
    }

    #[test]
    #[should_panic(expected = "server port must be positive")]
    fn setting_port_zero_panics() {
        let mut address: ServerAddress = "irc.example.net".parse().unwrap();
        address.set_port(0);
    }
}
