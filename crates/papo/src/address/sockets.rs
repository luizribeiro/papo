use super::{Endpoint, Protocol};

impl Endpoint {
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
    pub fn plaintext(host: impl Into<String>, port: u16) -> Result<Self, crate::InvalidUrl> {
        Self::tcp(host, port, Protocol::Irc)
    }

    pub(super) fn tcp(
        host: impl Into<String>,
        port: u16,
        protocol: Protocol,
    ) -> Result<Self, crate::InvalidUrl> {
        let host = host.into();
        let scheme = match protocol {
            Protocol::Irc => "irc",
            Protocol::Ircs => "ircs",
            Protocol::Ws | Protocol::Wss => unreachable!("TCP constructor used for WebSocket"),
        };
        let (open, close) = if host.contains(':') {
            ("[", "]")
        } else {
            ("", "")
        };
        format!("{scheme}://{open}{host}{close}:{port}").parse()
    }
}
