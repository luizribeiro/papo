use std::{fmt, str::FromStr};

use thiserror::Error;

const REJECTED_ADDRESS_CHARACTERS: [char; 5] = ['?', '#', '@', ' ', '\t'];

/// An invalid client configuration value.
#[derive(Clone, Debug, Eq, Error, Hash, PartialEq)]
#[non_exhaustive]
pub enum InvalidInput {
    /// A server URL is malformed or unsupported.
    #[error(transparent)]
    Url(#[from] crate::InvalidUrl),
    /// TLS trust, identity, or server-name input is invalid.
    #[error("invalid TLS configuration: {0}")]
    TlsConfig(String),
    /// A capability cannot be removed because papo does not request it by default.
    #[error("papo does not request capability {0:?} by default")]
    UnknownCapability(String),
    /// The event queue capacity is zero.
    #[error("event queue capacity must be positive")]
    ZeroEventCapacity,
    /// A rate-limit configuration is invalid.
    #[error(transparent)]
    RateLimit(#[from] crate::InvalidRateLimit),
    /// A byte secret was supplied to a text-only protocol field.
    #[error("byte secret cannot be used as text")]
    BinarySecret,
    /// Credentials were configured for an unencrypted endpoint without an explicit opt-in.
    #[error("credentials on a plaintext endpoint require allow_insecure_auth(true)")]
    InsecureAuthentication,
    /// An outbound IRC message cannot be represented on the wire.
    #[error("invalid IRC message: {0}")]
    WireMessage(#[source] crate::BuildError),
    /// A CTCP command or text contains a framing or IRC line delimiter.
    #[error("invalid CTCP command or text")]
    InvalidCtcp,
    /// A client-only message tag key does not begin with `+`.
    #[error("client-only message tag {0:?} must start with '+'")]
    InvalidClientTagKey(String),
    /// A custom stream cannot be reused for another bouncer network connection.
    #[error("a custom stream cannot be copied to a bouncer network connection")]
    CustomStreamNotReusable,
    /// A mode change is not valid under the server's advertised mode rules.
    #[error("invalid mode change: {0}")]
    InvalidMode(String),
    /// A metadata key is invalid.
    #[error("metadata key {key:?} is invalid: {reason}")]
    #[non_exhaustive]
    InvalidMetadataKey {
        /// The rejected metadata key.
        key: String,
        /// Why the protocol rejected the key.
        #[source]
        reason: crate::InvalidIdentifier,
    },
    /// A request contains input unsupported by its protocol command.
    #[error("invalid request input: {0}")]
    InvalidRequest(String),
    /// An IRC name is invalid in its protocol context.
    #[error("{argument} `{name}` is not a valid IRC name: {reason}")]
    #[non_exhaustive]
    InvalidName {
        /// The operation and argument role of the rejected name.
        argument: &'static str,
        /// The rejected name.
        name: String,
        /// Why the protocol rejected the name.
        #[source]
        reason: crate::InvalidIdentifier,
    },
    /// Outgoing chat text has no nonempty lines.
    #[error("chat text is empty")]
    EmptyText,
    /// Reaction text exceeds the IRC client-tag data limit.
    #[error("reaction length {actual} exceeds client-tag limit {maximum}")]
    #[non_exhaustive]
    Reaction {
        /// The `IRCv3` client-tag data limit.
        maximum: usize,
        /// The supplied reaction length in bytes.
        actual: usize,
    },
    /// A message reference lacks a server message identifier.
    #[error("the message reference has no server identifier")]
    UnusableMessageReference,
    /// A value belongs to another client.
    #[error("the value belongs to another client")]
    ForeignReference,
    /// An account operation omitted a required email address.
    #[error("account registration requires an email address")]
    EmailRequired,
    /// The server requires an account name to match the current nickname.
    #[error("the server does not support a custom account name")]
    AccountNameMustMatchNick,
    /// An account extended-ban name is empty, a logout marker, a glob, or invalid on the wire.
    #[error("invalid account extended-ban name {0:?}")]
    InvalidExtbanAccount(String),
    /// A kick reason exceeds the server's advertised byte limit.
    #[error("kick reason length {actual} exceeds limit {maximum}")]
    #[non_exhaustive]
    KickReasonTooLong {
        /// The server-advertised maximum length.
        maximum: usize,
        /// The supplied length in bytes.
        actual: usize,
    },
    /// A metadata value exceeds the server's advertised byte limit.
    #[error("metadata value length {actual} exceeds limit {maximum}")]
    #[non_exhaustive]
    MetadataValueTooLong {
        /// The advertised maximum length.
        maximum: usize,
        /// The supplied length.
        actual: usize,
    },
    /// A metadata subscription exceeds the server's advertised limit.
    #[error("metadata subscription count {actual} exceeds limit {maximum}")]
    #[non_exhaustive]
    TooManyMetadataSubscriptions {
        /// The advertised maximum count.
        maximum: usize,
        /// The supplied count.
        actual: usize,
    },
    /// A relayed nickname lacks an advertised separator.
    #[cfg(feature = "unstable")]
    #[error("relayed nickname has no allowed separator")]
    InvalidRelayNickname,
    /// A Web Push endpoint is not a nonempty HTTPS URL.
    #[cfg(feature = "unstable")]
    #[error("invalid Web Push endpoint")]
    InvalidWebPushEndpoint,
    /// A Web Push key is not valid URL-safe base64.
    #[cfg(feature = "unstable")]
    #[error("invalid Web Push key")]
    InvalidWebPushKey,
}

impl InvalidInput {
    pub(crate) fn invalid_name(name: impl Into<String>, reason: crate::InvalidIdentifier) -> Self {
        Self::InvalidName {
            argument: "name",
            name: name.into(),
            reason,
        }
    }

    pub(crate) fn invalid_argument(
        argument: &'static str,
        name: impl Into<String>,
        reason: crate::InvalidIdentifier,
    ) -> Self {
        Self::InvalidName {
            argument,
            name: name.into(),
            reason,
        }
    }
}

/// The network endpoint used by an IRC client.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Endpoint(ServerAddress);

/// The transport and security used by an [`Endpoint`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum EndpointKind {
    /// Unencrypted IRC over TCP.
    Plaintext,
    /// IRC over TLS.
    Tls,
    /// An unencrypted WebSocket.
    WebSocket,
    /// A WebSocket protected by TLS.
    SecureWebSocket,
}

impl Endpoint {
    /// Creates a TLS-protected TCP endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when `host` is empty or malformed, or `port` is zero.
    ///
    /// ```
    /// let endpoint = papo::Endpoint::tls("irc.example.net", 6697);
    /// # _ = endpoint;
    /// ```
    ///
    #[cfg(sockets)]
    pub fn tls(host: impl Into<String>, port: u16) -> Result<Self, InvalidInput> {
        Self::tcp(host, port, Protocol::Ircs)
    }

    /// Creates an unencrypted TCP endpoint that remains eligible for STS upgrades.
    ///
    /// # Errors
    ///
    /// Returns an error when `host` is empty or malformed, or `port` is zero.
    ///
    /// ```
    /// let endpoint = papo::Endpoint::plaintext("irc.example.net", 6667);
    /// # _ = endpoint;
    /// ```
    ///
    #[cfg(sockets)]
    pub fn plaintext(host: impl Into<String>, port: u16) -> Result<Self, InvalidInput> {
        Self::tcp(host, port, Protocol::Irc)
    }

    /// Creates a WebSocket endpoint from a `ws://` or `wss://` URL.
    ///
    /// # Errors
    ///
    /// Returns an error when the URL has the wrong scheme or a malformed host or port.
    ///
    /// ```
    /// let endpoint = papo::Endpoint::websocket("wss://irc.example.net/webirc");
    /// # _ = endpoint;
    /// ```
    ///
    pub fn websocket(url: impl AsRef<str>) -> Result<Self, InvalidInput> {
        let url = url.as_ref();
        let Some((scheme, _)) = url.split_once("://") else {
            return Err(crate::InvalidUrl::MissingScheme.into());
        };
        if !scheme.eq_ignore_ascii_case("ws") && !scheme.eq_ignore_ascii_case("wss") {
            return Err(crate::InvalidUrl::UnsupportedScheme(scheme.to_owned()).into());
        }
        url.parse().map(Self)
    }

    /// Returns the endpoint's hostname.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.0.host
    }

    /// Returns the endpoint's service port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.0.port
    }

    /// Returns the endpoint's transport kind.
    #[must_use]
    pub fn kind(&self) -> EndpointKind {
        match self.0.protocol {
            Protocol::Irc => EndpointKind::Plaintext,
            Protocol::Ircs => EndpointKind::Tls,
            Protocol::Ws => EndpointKind::WebSocket,
            Protocol::Wss => EndpointKind::SecureWebSocket,
        }
    }

    #[cfg(sockets)]
    fn tcp(host: impl Into<String>, port: u16, protocol: Protocol) -> Result<Self, InvalidInput> {
        let host = validate_host(host.into())?;
        if port == 0 {
            return Err(crate::InvalidUrl::InvalidPort(port.to_string()).into());
        }
        Ok(Self(ServerAddress {
            protocol,
            host,
            port,
            explicit_port: true,
            websocket_path: None,
        }))
    }

    pub(crate) fn into_address(self) -> ServerAddress {
        self.0
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

mod private {
    use super::{Endpoint, InvalidInput};

    pub trait Sealed {
        fn into_endpoint(self) -> Result<Endpoint, InvalidInput>;
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
    fn into_endpoint(self) -> Result<Endpoint, InvalidInput> {
        Ok(self)
    }
}

impl private::Sealed for &str {
    fn into_endpoint(self) -> Result<Endpoint, InvalidInput> {
        self.parse().map(Endpoint)
    }
}

impl private::Sealed for String {
    fn into_endpoint(self) -> Result<Endpoint, InvalidInput> {
        self.parse().map(Endpoint)
    }
}

pub(crate) fn convert(value: impl IntoEndpoint) -> Result<ServerAddress, InvalidInput> {
    private::Sealed::into_endpoint(value).map(Endpoint::into_address)
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
        use papo_core::server_url::ServerUrlScheme;

        let url = input
            .parse::<papo_core::server_url::ServerUrl>()
            .map_err(InvalidInput::from)?;
        let protocol = match url.scheme() {
            ServerUrlScheme::Irc => Protocol::Irc,
            ServerUrlScheme::Ircs => Protocol::Ircs,
            ServerUrlScheme::Ws => Protocol::Ws,
            ServerUrlScheme::Wss => Protocol::Wss,
            scheme => {
                return Err(crate::InvalidUrl::UnsupportedScheme(scheme.to_string()).into());
            }
        };
        Ok(Self {
            protocol,
            host: url.host().to_owned(),
            port: url.port(),
            explicit_port: url.has_explicit_port(),
            websocket_path: url.path().map(str::to_owned),
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
        self.protocol = match (self.is_websocket(), enabled) {
            (false, false) => Protocol::Irc,
            (false, true) => Protocol::Ircs,
            (true, false) => Protocol::Ws,
            (true, true) => Protocol::Wss,
        };
        if !self.explicit_port {
            self.port = default_port(self.protocol);
        }
    }

    pub(crate) const fn is_websocket(&self) -> bool {
        matches!(self.protocol, Protocol::Ws | Protocol::Wss)
    }

    pub(crate) fn apply_sts_upgrade(&mut self, advertised_port: u16) {
        let websocket = self.is_websocket();
        self.set_tls(true);
        if !websocket {
            self.set_port(advertised_port);
        }
    }

    pub(crate) fn apply_stored_sts(&mut self, policy: &papo_core::sts::StsPolicy) {
        let websocket = self.is_websocket();
        self.set_tls(true);
        if websocket == policy.websocket {
            self.set_port(policy.port);
        }
    }

    pub(crate) fn is_secure(&self) -> bool {
        matches!(self.protocol, Protocol::Ircs | Protocol::Wss)
    }

    #[cfg(any(sockets, browser))]
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

impl fmt::Display for ServerAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = match self.protocol {
            Protocol::Irc => "irc",
            Protocol::Ircs => "ircs",
            Protocol::Ws => "ws",
            Protocol::Wss => "wss",
        };
        let (open, close) = if self.host.contains(':') {
            ("[", "]")
        } else {
            ("", "")
        };
        write!(formatter, "{scheme}://{open}{}{close}", self.host)?;
        if self.explicit_port {
            write!(formatter, ":{}", self.port)?;
        }
        if let Some(path) = &self.websocket_path {
            formatter.write_str(path)?;
        }
        Ok(())
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

fn validate_host(host: String) -> Result<String, InvalidInput> {
    if host.is_empty() || host.contains(REJECTED_ADDRESS_CHARACTERS) || host.contains(['[', ']']) {
        return Err(crate::InvalidUrl::InvalidHost(host).into());
    }
    Ok(host)
}

#[cfg(all(test, not(browser)))]
mod tests {
    use super::{Endpoint, EndpointKind, InvalidInput, Protocol, ServerAddress};

    #[test]
    fn wraps_rate_limit_errors() {
        let error = InvalidInput::from(crate::InvalidRateLimit::ZeroRefill);
        assert_eq!(
            error,
            InvalidInput::RateLimit(crate::InvalidRateLimit::ZeroRefill)
        );
        assert_eq!(error.to_string(), "rate-limit refill must be positive");
    }

    #[test]
    fn typed_constructors_preserve_transport_details() {
        let tls = Endpoint::tls("irc.example.net", 7443).unwrap().0;
        assert_eq!(
            (tls.protocol, tls.host.as_str(), tls.port),
            (Protocol::Ircs, "irc.example.net", 7443)
        );

        let plain = Endpoint::plaintext("irc.example.net", 7000).unwrap().0;
        assert_eq!(
            (plain.protocol, plain.host.as_str(), plain.port),
            (Protocol::Irc, "irc.example.net", 7000)
        );

        let websocket = Endpoint::websocket("wss://irc.example.net:8443/webirc")
            .unwrap()
            .0;
        assert_eq!(
            (websocket.protocol, websocket.host.as_str(), websocket.port),
            (Protocol::Wss, "irc.example.net", 8443)
        );
        assert_eq!(websocket.websocket_path.as_deref(), Some("/webirc"));
    }

    #[test]
    fn endpoint_accessors_and_display_preserve_configuration() {
        let tls = Endpoint::tls("2001:db8::7", 6697).unwrap();
        assert_eq!(tls.host(), "2001:db8::7");
        assert_eq!(tls.port(), 6697);
        assert_eq!(tls.kind(), EndpointKind::Tls);
        assert_eq!(tls.to_string(), "ircs://[2001:db8::7]:6697");

        let socket = Endpoint::websocket("ws://irc.example.net/webirc").unwrap();
        assert_eq!(socket.host(), "irc.example.net");
        assert_eq!(socket.port(), 80);
        assert_eq!(socket.kind(), EndpointKind::WebSocket);
        assert_eq!(socket.to_string(), "ws://irc.example.net/webirc");
    }

    #[test]
    fn websocket_rejects_non_websocket_schemes_and_userinfo() {
        for url in [
            "irc.example.net",
            "irc://irc.example.net",
            "https://irc.example.net/webirc",
            "wss://alice@irc.example.net/webirc",
        ] {
            assert!(Endpoint::websocket(url).is_err(), "accepted {url:?}");
        }
    }

    #[test]
    fn typed_endpoints_reject_invalid_input_at_construction() {
        for endpoint in [
            Endpoint::tls("irc.example.net", 0),
            Endpoint::plaintext("irc.example.net", 0),
        ] {
            assert_eq!(
                endpoint,
                Err(InvalidInput::Url(crate::InvalidUrl::InvalidPort(
                    "0".into()
                )))
            );
        }
        for host in ["", "irc example.net"] {
            assert_eq!(
                Endpoint::tls(host, 6697),
                Err(InvalidInput::Url(crate::InvalidUrl::InvalidHost(
                    host.into()
                )))
            );
        }
    }

    #[test]
    fn parses_every_server_address_form() {
        let cases = [
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
            (
                "ircs://[2001:db8::1]:6697",
                Protocol::Ircs,
                "2001:db8::1",
                6697,
            ),
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
                address.is_websocket(),
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
            InvalidInput::Url(crate::InvalidUrl::UnsupportedScheme("ftp".into()))
        );
        assert_eq!("irc://host:1".parse::<ServerAddress>().unwrap().port, 1);
        assert_eq!(
            "irc://host:65535".parse::<ServerAddress>().unwrap().port,
            65535
        );
    }

    #[test]
    fn rejects_url_components_and_ipv6_suffixes_with_typed_errors() {
        for input in ["irc://alice@irc.example.net", "irc://[::1]x"] {
            assert!(
                matches!(
                    input.parse::<ServerAddress>(),
                    Err(InvalidInput::Url(
                        crate::InvalidUrl::Userinfo | crate::InvalidUrl::InvalidHost(_)
                    ))
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
    fn sts_upgrade_preserves_the_websocket_transport_port() {
        let mut address: ServerAddress = "ws://irc.example.net/webirc".parse().unwrap();
        address.apply_sts_upgrade(6697);
        assert_eq!(address.to_string(), "wss://irc.example.net/webirc");
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
    fn preserves_each_shared_parser_error() {
        for input in [
            "irc://irc.example.net?network=libera",
            "irc://irc.example.net#network",
            "ws://irc example.net/irc",
            "ws://irc.example.net/irc\tgateway",
        ] {
            assert!(matches!(
                input.parse::<ServerAddress>(),
                Err(InvalidInput::Url(
                    crate::InvalidUrl::Query
                        | crate::InvalidUrl::Fragment
                        | crate::InvalidUrl::Whitespace
                        | crate::InvalidUrl::InvalidHost(_)
                ))
            ));
        }
    }

    #[test]
    #[should_panic(expected = "server port must be positive")]
    fn setting_port_zero_panics() {
        let mut address: ServerAddress = "irc://irc.example.net".parse().unwrap();
        address.set_port(0);
    }
}
