use std::str::FromStr;

use thiserror::Error;

/// An error in an IRC server address.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum AddressError {
    /// The address is empty.
    #[error("server address is empty")]
    Empty,
    /// The URL scheme is not supported.
    #[error("unsupported scheme {0:?}")]
    UnsupportedScheme(String),
    /// The host or bracketed IPv6 literal is malformed.
    #[error("server address has an invalid host")]
    InvalidHost,
    /// The service port is missing, zero, or greater than 65535.
    #[error("invalid service port {0:?}")]
    InvalidPort(String),
    /// A TCP IRC URL contains a path, query, or fragment.
    #[error("IRC server address cannot contain a path")]
    UnexpectedPath,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Protocol {
    Irc,
    Ircs,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServerAddress {
    pub(crate) protocol: Protocol,
    pub(crate) host: String,
    pub(crate) port: u16,
}

impl FromStr for ServerAddress {
    type Err = AddressError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.is_empty() {
            return Err(AddressError::Empty);
        }
        let (protocol, remainder) = match input.split_once("://") {
            Some(("irc", rest)) => (Protocol::Irc, rest),
            Some(("ircs", rest)) => (Protocol::Ircs, rest),
            Some((scheme, _)) => return Err(AddressError::UnsupportedScheme(scheme.to_owned())),
            None => (Protocol::Ircs, input),
        };
        if remainder.contains('/') {
            return Err(AddressError::UnexpectedPath);
        }
        if remainder.contains(['?', '#', '@']) {
            return Err(AddressError::InvalidHost);
        }
        let default_port = match protocol {
            Protocol::Irc => 6667,
            Protocol::Ircs => 6697,
        };
        let (host, port) = parse_authority(remainder, default_port)?;
        Ok(Self {
            protocol,
            host,
            port,
        })
    }
}

fn parse_authority(authority: &str, default_port: u16) -> Result<(String, u16), AddressError> {
    if authority.matches(':').count() > 1 {
        return Err(AddressError::InvalidHost);
    }
    let (host, port) = authority
        .split_once(':')
        .map_or((authority, None), |(host, port)| (host, Some(port)));
    if host.is_empty() || host.contains(['[', ']', ' ', '\t']) {
        return Err(AddressError::InvalidHost);
    }
    let port = port.map_or(Ok(default_port), |port| {
        port.parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| AddressError::InvalidPort(port.to_owned()))
    })?;
    Ok((host.to_owned(), port))
}

#[cfg(test)]
mod tests {
    use super::{AddressError, Protocol, ServerAddress};

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
        ];
        for (input, protocol, host, port) in cases {
            let address: ServerAddress = input.parse().unwrap();
            assert_eq!(
                (address.protocol, address.host.as_str(), address.port),
                (protocol, host, port)
            );
        }
    }

    #[test]
    fn rejects_malformed_addresses_at_each_boundary() {
        for input in [
            "",
            "irc://",
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
            AddressError::UnsupportedScheme("ftp".into())
        );
        assert_eq!("host:1".parse::<ServerAddress>().unwrap().port, 1);
        assert_eq!("host:65535".parse::<ServerAddress>().unwrap().port, 65535);
    }
}
