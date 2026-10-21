use std::net::IpAddr;

/// Identity asserted by a trusted IRC gateway with `WEBIRC`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebIrc {
    /// Password shared with the IRC server.
    pub password: crate::Secret,
    /// Gateway name configured on the IRC server.
    pub gateway: String,
    /// Client IP address.
    pub client_ip: IpAddr,
    /// Client hostname, or `None` to send the client's IP address as its hostname.
    pub client_hostname: Option<String>,
    /// Ordered `name[=value]` options appended to the command.
    pub options: Vec<(String, Option<String>)>,
}

impl WebIrc {
    pub(crate) fn into_core(self) -> papo_core::WebIrc {
        papo_core::WebIrc {
            password: self.password,
            gateway: self.gateway,
            client_ip: self.client_ip,
            client_hostname: self.client_hostname,
            options: self.options,
        }
    }
}
