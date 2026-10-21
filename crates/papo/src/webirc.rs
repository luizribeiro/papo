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
    #[doc(hidden)]
    #[must_use]
    pub fn new(
        password: impl Into<crate::Secret>,
        gateway: impl Into<String>,
        client_ip: impl AsRef<str>,
    ) -> Self {
        Self {
            password: password.into(),
            gateway: gateway.into(),
            client_ip: client_ip.as_ref().parse().expect("valid WEBIRC client IP"),
            client_hostname: None,
            options: Vec::new(),
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn hostname(mut self, hostname: impl Into<String>) -> Self {
        self.client_hostname = Some(hostname.into());
        self
    }

    #[doc(hidden)]
    #[must_use]
    pub fn option(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.push((name.into(), Some(value.into())));
        self
    }

    #[doc(hidden)]
    #[must_use]
    pub fn flag(mut self, name: impl Into<String>) -> Self {
        self.options.push((name.into(), None));
        self
    }

    pub(crate) fn into_core(self) -> papo_core::WebIrc {
        papo_core::WebIrc {
            password: self.password,
            gateway: self.gateway,
            hostname: self.client_hostname,
            ip: self.client_ip.to_string(),
            options: self.options,
        }
    }
}
