use std::net::IpAddr;

/// Identity asserted by a trusted IRC gateway with `WEBIRC`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct WebIrc {
    password: crate::Secret,
    gateway: String,
    client_ip: IpAddr,
    client_hostname: Option<String>,
    options: Vec<(String, Option<String>)>,
}

impl WebIrc {
    /// Creates a trusted gateway identity for `client_ip`.
    pub fn new(
        gateway: impl Into<String>,
        password: impl Into<crate::Secret>,
        client_ip: IpAddr,
    ) -> Self {
        Self {
            password: password.into(),
            gateway: gateway.into(),
            client_ip,
            client_hostname: None,
            options: Vec::new(),
        }
    }

    /// Sets the asserted client hostname.
    #[must_use]
    pub fn hostname(mut self, hostname: impl Into<String>) -> Self {
        self.client_hostname = Some(hostname.into());
        self
    }

    /// Appends a valueless WEBIRC option.
    #[must_use]
    pub fn flag(mut self, name: impl Into<String>) -> Self {
        self.options.push((name.into(), None));
        self
    }

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
