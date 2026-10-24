use std::{fmt, net::Ipv6Addr, str::FromStr};

/// A transport scheme accepted in an IRC server URL.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ServerUrlScheme {
    /// Unencrypted IRC over TCP.
    Irc,
    /// IRC over TLS.
    Ircs,
    /// Unencrypted IRC over WebSocket.
    Ws,
    /// IRC over a TLS-protected WebSocket.
    Wss,
}

impl ServerUrlScheme {
    /// Returns the canonical lowercase spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Irc => "irc",
            Self::Ircs => "ircs",
            Self::Ws => "ws",
            Self::Wss => "wss",
        }
    }

    /// Returns the scheme's default service port.
    #[must_use]
    pub const fn default_port(self) -> u16 {
        match self {
            Self::Irc => 6667,
            Self::Ircs => 6697,
            Self::Ws => 80,
            Self::Wss => 443,
        }
    }

    /// Reports whether this scheme permits a URL path.
    #[must_use]
    pub const fn permits_path(self) -> bool {
        matches!(self, Self::Ws | Self::Wss)
    }
}

impl fmt::Display for ServerUrlScheme {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A validated IRC server URL.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ServerUrl {
    pub(crate) scheme: ServerUrlScheme,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) explicit_port: bool,
    pub(crate) path: Option<String>,
}

impl ServerUrl {
    /// Returns the transport scheme.
    #[must_use]
    pub const fn scheme(&self) -> ServerUrlScheme {
        self.scheme
    }

    /// Returns the normalized host without IPv6 brackets.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns the explicit or scheme-default port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Reports whether the input included a port.
    #[must_use]
    pub const fn has_explicit_port(&self) -> bool {
        self.explicit_port
    }

    /// Returns a WebSocket path when one was present.
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
}

impl fmt::Display for ServerUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}://", self.scheme)?;
        if self.host.contains(':') {
            write!(formatter, "[{}]", self.host)?;
        } else {
            formatter.write_str(&self.host)?;
        }
        if self.explicit_port {
            write!(formatter, ":{}", self.port)?;
        }
        if let Some(path) = &self.path {
            formatter.write_str(path)?;
        }
        Ok(())
    }
}

/// Why an IRC server URL could not be parsed.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum InvalidServerUrl {
    /// The URL is empty.
    Empty,
    /// The URL has no scheme separator.
    MissingScheme,
    /// The URL scheme is not supported.
    UnsupportedScheme(String),
    /// The URL has no host.
    EmptyHost,
    /// The URL contains user information.
    Userinfo,
    /// The URL contains a query.
    Query,
    /// The URL contains a fragment.
    Fragment,
    /// The URL contains whitespace.
    Whitespace,
    /// The URL contains a control character.
    ControlCharacter,
    /// The URL contains percent-encoding.
    PercentEncoding,
    /// The host is non-ASCII and must be supplied in punycode form.
    NonAsciiHost(String),
    /// The host ends in a dot.
    TrailingDot(String),
    /// An IPv6 literal contains a zone identifier.
    ZoneIdentifier(String),
    /// An IPv6 literal is missing brackets.
    UnbracketedIpv6(String),
    /// A host or bracketed IPv6 literal is malformed.
    InvalidHost(String),
    /// The port is missing, zero, malformed, or greater than 65535.
    InvalidPort(String),
    /// A non-WebSocket URL contains a path.
    UnexpectedPath(String),
}

impl fmt::Display for InvalidServerUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("server URL is empty"),
            Self::MissingScheme => formatter.write_str("server URL scheme is missing"),
            Self::UnsupportedScheme(value) => write!(formatter, "unsupported scheme {value:?}"),
            Self::EmptyHost => formatter.write_str("server URL host is empty"),
            Self::Userinfo => formatter.write_str("server URL cannot contain user information"),
            Self::Query => formatter.write_str("server URL cannot contain a query"),
            Self::Fragment => formatter.write_str("server URL cannot contain a fragment"),
            Self::Whitespace => formatter.write_str("server URL cannot contain whitespace"),
            Self::ControlCharacter => {
                formatter.write_str("server URL cannot contain control characters")
            }
            Self::PercentEncoding => {
                formatter.write_str("server URL cannot contain percent-encoding")
            }
            Self::NonAsciiHost(value) => write!(
                formatter,
                "non-ASCII server host {value:?}; use its punycode xn-- form"
            ),
            Self::TrailingDot(value) => {
                write!(formatter, "server host {value:?} has a trailing dot")
            }
            Self::ZoneIdentifier(value) => {
                write!(
                    formatter,
                    "IPv6 zone identifiers are not supported in {value:?}"
                )
            }
            Self::UnbracketedIpv6(value) => {
                write!(
                    formatter,
                    "IPv6 literal {value:?} must be enclosed in brackets"
                )
            }
            Self::InvalidHost(value) => write!(formatter, "invalid server host {value:?}"),
            Self::InvalidPort(value) => write!(formatter, "invalid service port {value:?}"),
            Self::UnexpectedPath(value) => {
                write!(formatter, "IRC server URL cannot contain path {value:?}")
            }
        }
    }
}

impl std::error::Error for InvalidServerUrl {}
impl FromStr for ServerUrl {
    type Err = InvalidServerUrl;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.is_empty() {
            return Err(InvalidServerUrl::Empty);
        }
        if input.chars().any(char::is_whitespace) {
            return Err(InvalidServerUrl::Whitespace);
        }
        if input.chars().any(char::is_control) {
            return Err(InvalidServerUrl::ControlCharacter);
        }
        let Some((scheme, remainder)) = input.split_once("://") else {
            return Err(InvalidServerUrl::MissingScheme);
        };
        let scheme = parse_scheme(scheme)?;
        if remainder.contains('#') {
            return Err(InvalidServerUrl::Fragment);
        }
        if remainder.contains('?') {
            return Err(InvalidServerUrl::Query);
        }
        let (authority, path) = match remainder.split_once('/') {
            Some((authority, path)) => (authority, Some(format!("/{path}"))),
            None => (remainder, None),
        };
        if let Some(path) = &path
            && !scheme.permits_path()
        {
            return Err(InvalidServerUrl::UnexpectedPath(path.clone()));
        }
        if authority.contains('@') {
            return Err(InvalidServerUrl::Userinfo);
        }
        if authority.is_empty() {
            return Err(InvalidServerUrl::EmptyHost);
        }
        let (host, port, explicit_port) = parse_authority(authority, scheme.default_port())?;
        if path.as_deref().is_some_and(|path| path.contains('%')) {
            return Err(InvalidServerUrl::PercentEncoding);
        }
        Ok(Self {
            scheme,
            host,
            port,
            explicit_port,
            path,
        })
    }
}
fn parse_scheme(value: &str) -> Result<ServerUrlScheme, InvalidServerUrl> {
    if value.eq_ignore_ascii_case("irc") {
        Ok(ServerUrlScheme::Irc)
    } else if value.eq_ignore_ascii_case("ircs") {
        Ok(ServerUrlScheme::Ircs)
    } else if value.eq_ignore_ascii_case("ws") {
        Ok(ServerUrlScheme::Ws)
    } else if value.eq_ignore_ascii_case("wss") {
        Ok(ServerUrlScheme::Wss)
    } else {
        Err(InvalidServerUrl::UnsupportedScheme(value.to_owned()))
    }
}
fn parse_authority(
    authority: &str,
    default_port: u16,
) -> Result<(String, u16, bool), InvalidServerUrl> {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let Some((host, suffix)) = rest.split_once(']') else {
            return Err(InvalidServerUrl::InvalidHost(authority.to_owned()));
        };
        if host.contains('%') {
            return Err(InvalidServerUrl::ZoneIdentifier(host.to_owned()));
        }
        let host = host
            .parse::<Ipv6Addr>()
            .map_err(|_| InvalidServerUrl::InvalidHost(host.to_owned()))?
            .to_string();
        let port = if suffix.is_empty() {
            None
        } else {
            Some(
                suffix
                    .strip_prefix(':')
                    .ok_or_else(|| InvalidServerUrl::InvalidHost(authority.to_owned()))?,
            )
        };
        (host, port)
    } else {
        if authority.matches(':').count() > 1 {
            return Err(InvalidServerUrl::UnbracketedIpv6(authority.to_owned()));
        }
        let (host, port) = authority
            .split_once(':')
            .map_or((authority, None), |(host, port)| (host, Some(port)));
        (validate_hostname(host)?, port)
    };
    let Some(port) = port else {
        return Ok((host, default_port, false));
    };
    let parsed = port
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| InvalidServerUrl::InvalidPort(port.to_owned()))?;
    Ok((host, parsed, true))
}
fn validate_hostname(host: &str) -> Result<String, InvalidServerUrl> {
    if host.is_empty() {
        return Err(InvalidServerUrl::EmptyHost);
    }
    if !host.is_ascii() {
        return Err(InvalidServerUrl::NonAsciiHost(host.to_owned()));
    }
    if host.contains('%') {
        return Err(InvalidServerUrl::PercentEncoding);
    }
    if host.ends_with('.') {
        return Err(InvalidServerUrl::TrailingDot(host.to_owned()));
    }
    let valid = host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.starts_with(|character: char| character.is_ascii_alphanumeric())
                && label.ends_with(|character: char| character.is_ascii_alphanumeric())
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    if !valid || host.contains(['[', ']']) {
        return Err(InvalidServerUrl::InvalidHost(host.to_owned()));
    }
    Ok(host.to_owned())
}
