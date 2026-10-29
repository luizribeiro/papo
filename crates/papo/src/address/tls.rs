use super::{Endpoint, Protocol};

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
    pub fn tls(host: impl Into<String>, port: u16) -> Result<Self, crate::InvalidUrl> {
        Self::tcp(host, port, Protocol::Ircs)
    }
}
