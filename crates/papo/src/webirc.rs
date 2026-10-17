/// Identity asserted by a trusted IRC gateway with `WEBIRC`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebIrc {
    inner: papo_core::WebIrc,
}

impl WebIrc {
    /// Creates a gateway identity for a client IP address.
    #[must_use]
    pub fn new(
        password: impl Into<crate::Secret>,
        gateway: impl Into<String>,
        ip: impl Into<String>,
    ) -> Self {
        Self {
            inner: papo_core::WebIrc {
                password: password.into(),
                gateway: gateway.into(),
                hostname: None,
                ip: ip.into(),
                options: Vec::new(),
            },
        }
    }

    /// Supplies the client's hostname when it differs from its IP address.
    #[must_use]
    pub fn hostname(mut self, hostname: impl Into<String>) -> Self {
        self.inner.hostname = Some(hostname.into());
        self
    }

    /// Appends a valued `WEBIRC` option.
    #[must_use]
    pub fn option(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.inner.options.push((name.into(), Some(value.into())));
        self
    }

    /// Appends a valueless `WEBIRC` option.
    #[must_use]
    pub fn flag(mut self, name: impl Into<String>) -> Self {
        self.inner.options.push((name.into(), None));
        self
    }

    pub(crate) fn into_core(self) -> papo_core::WebIrc {
        self.inner
    }
}
