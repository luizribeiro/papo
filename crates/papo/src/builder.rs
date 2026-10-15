use std::{path::Path, time::Duration};

use papo_core::sts::ConnectionSecurity;
use papo_core::{Config, RateLimit, Sasl};
#[cfg(feature = "spawn")]
use tokio::sync::oneshot;

use crate::{
    AddressError, Error, Reconnect,
    address::ServerAddress,
    driver::{ConnectionPlan, PostRegistration},
    transport::{ClientCertificate, TlsConfig, TlsRoots},
};

/// Configures an IRC client before it is built or connected.
#[must_use]
pub struct Builder {
    pub(crate) address: Result<ServerAddress, AddressError>,
    pub(crate) config: Config,
    pub(crate) event_capacity: usize,
    pub(crate) tls: TlsConfig,
    reconnect: Reconnect,
    disconnected_queue_capacity: usize,
    bot: bool,
    channels: Vec<String>,
    username_set: bool,
    realname_set: bool,
}

impl Builder {
    pub(crate) fn new(address: &str) -> Self {
        Self {
            address: address.parse(),
            config: Config::default(),
            event_capacity: crate::driver::DEFAULT_CHANNEL_CAPACITY,
            tls: TlsConfig::default(),
            reconnect: Reconnect::default(),
            disconnected_queue_capacity: 256,
            bot: false,
            channels: Vec::new(),
            username_set: false,
            realname_set: false,
        }
    }

    /// Sets the primary nickname.
    pub fn nick(mut self, nick: impl Into<String>) -> Self {
        let nick = nick.into();
        self.config.nick.clone_from(&nick);
        if !self.username_set {
            self.config.username.clone_from(&nick);
        }
        if !self.realname_set {
            self.config.realname.clone_from(&nick);
        }
        self
    }

    /// Sets alternative nicknames to try after a collision.
    pub fn alt_nicks<I, S>(mut self, nicks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.alt_nicks = nicks.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the username sent during registration.
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.config.username = username.into();
        self.username_set = true;
        self
    }

    /// Sets the real name sent during registration.
    pub fn realname(mut self, realname: impl Into<String>) -> Self {
        self.config.realname = realname.into();
        self.realname_set = true;
        self
    }

    /// Sets the server password sent with `PASS`.
    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.config.password = Some(password.into());
        self
    }

    /// Configures SASL authentication.
    pub fn sasl(mut self, sasl: Sasl) -> Self {
        self.config.sasl = Some(sasl);
        self
    }

    /// Sets whether successful SASL authentication is required.
    pub fn require_sasl(mut self, required: bool) -> Self {
        self.config.require_sasl = required;
        self
    }

    /// Sets the Ergo multiclient device identifier.
    pub fn device(mut self, device: impl Into<String>) -> Self {
        self.config.device = Some(device.into());
        self
    }

    /// Replaces the capability names requested when advertised.
    pub fn caps<I, S>(mut self, caps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.wanted_caps = caps.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the outbound line rate limit.
    pub fn rate_limit(mut self, rate_limit: RateLimit) -> Self {
        self.config.rate_limit = rate_limit;
        self
    }

    /// Sets the idle interval before sending a keepalive probe.
    pub fn ping_interval(mut self, interval: Duration) -> Self {
        self.config.ping_interval = interval;
        self
    }

    /// Sets the maximum time allowed without an incoming line.
    pub fn ping_timeout(mut self, timeout: Duration) -> Self {
        self.config.ping_timeout = timeout;
        self
    }

    /// Sets the bounded event queue capacity.
    ///
    /// # Panics
    ///
    /// Panics when `capacity` is zero.
    pub fn event_capacity(mut self, capacity: usize) -> Self {
        assert!(capacity > 0, "event capacity must be positive");
        self.event_capacity = capacity;
        self
    }

    /// Sets the policy for reconnecting after an unexpected disconnect.
    pub const fn reconnect(mut self, reconnect: Reconnect) -> Self {
        self.reconnect = reconnect;
        self
    }

    /// Sets the number of commands retained while no transport is connected.
    pub const fn disconnected_queue_capacity(mut self, capacity: usize) -> Self {
        self.disconnected_queue_capacity = capacity;
        self
    }

    /// Overrides the service port.
    pub fn port(mut self, port: u16) -> Self {
        if let Ok(address) = &mut self.address {
            address.set_port(port);
        }
        self
    }

    /// Selects whether the connection is protected by TLS.
    pub fn tls(mut self, enabled: bool) -> Self {
        if let Ok(address) = &mut self.address {
            address.set_tls(enabled);
        }
        self
    }

    /// Selects an explicitly unencrypted connection.
    pub fn plaintext(self) -> Self {
        self.tls(false)
    }

    /// Trusts only the certificates in a PEM document.
    pub fn tls_ca_pem(mut self, pem: impl Into<Vec<u8>>) -> Self {
        self.tls.roots = TlsRoots::Ca(pem.into());
        self
    }

    /// Reads a PEM trust document from `path`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ReadFile`] when the file cannot be read.
    pub fn tls_ca_file(self, path: impl AsRef<Path>) -> Result<Self, Error> {
        self.read_file(path)
            .map(|(builder, pem)| builder.tls_ca_pem(pem))
    }

    /// Sets a PEM-encoded TLS client certificate chain and private key.
    pub fn tls_client_cert(
        mut self,
        certificate: impl Into<Vec<u8>>,
        key: impl Into<Vec<u8>>,
    ) -> Self {
        self.tls.client_certificate = Some(ClientCertificate::new(certificate, key));
        self
    }

    /// Reads a TLS client certificate chain and private key from PEM files.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ReadFile`] when either file cannot be read.
    pub fn tls_client_cert_files(
        self,
        certificate: impl AsRef<Path>,
        key: impl AsRef<Path>,
    ) -> Result<Self, Error> {
        let (builder, certificate) = self.read_file(certificate)?;
        let (builder, key) = builder.read_file(key)?;
        Ok(builder.tls_client_cert(certificate, key))
    }

    /// Sets the server-advertised bot user mode after registration.
    pub fn bot(mut self) -> Self {
        self.bot = true;
        self
    }

    /// Sets channels to join after registration.
    pub fn channels<I, S>(mut self, channels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.channels = channels.into_iter().map(Into::into).collect();
        self
    }

    /// Builds handles and a lazy driver without opening a connection.
    ///
    /// The returned [`crate::Driver`] connects when it is first polled. Use
    /// [`crate::Driver::new`] to drive an already-connected custom transport.
    pub fn build(self) -> (crate::Client, crate::Events, crate::Driver) {
        self.build_with_registration(None)
    }

    /// Connects in a spawned driver and waits for IRC registration.
    ///
    /// The [`crate::Event::Registered`] event remains available from the
    /// returned event stream.
    ///
    /// # Errors
    ///
    /// Returns connection or registration errors that occur before welcome.
    #[cfg(feature = "spawn")]
    pub async fn connect(self) -> Result<(crate::Client, crate::Events), Error> {
        let (sender, receiver) = oneshot::channel();
        let (client, events, driver) = self.build_with_registration(Some(sender));
        tokio::spawn(driver);
        receiver.await.map_err(|_| Error::Disconnected)??;
        Ok((client, events))
    }

    fn build_with_registration(
        mut self,
        registration: Option<crate::driver::RegistrationSender>,
    ) -> (crate::Client, crate::Events, crate::Driver) {
        let plan = self.address.map(|address| {
            apply_address(&mut self.config, &address);
            ConnectionPlan {
                address,
                tls: self.tls,
            }
        });
        let session = papo_core::Session::with_entropy(self.config, Box::new(crate::OsEntropy));
        let defaults = PostRegistration {
            bot: self.bot,
            channels: self.channels,
        };
        crate::Driver::with_plan(
            plan,
            session,
            self.event_capacity,
            registration,
            defaults,
            self.reconnect,
            self.disconnected_queue_capacity,
        )
    }

    fn read_file(self, path: impl AsRef<Path>) -> Result<(Self, Vec<u8>), Error> {
        let path = path.as_ref();
        std::fs::read(path)
            .map(|contents| (self, contents))
            .map_err(|source| Error::ReadFile {
                path: path.to_owned(),
                source,
            })
    }
}

fn apply_address(config: &mut Config, address: &ServerAddress) {
    config.host.clone_from(&address.host);
    config.connection_security = if address.is_secure() {
        ConnectionSecurity::Secure
    } else {
        ConnectionSecurity::Plaintext
    };
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use papo_core::{RateLimit, Sasl, sts::ConnectionSecurity};

    use super::{Builder, apply_address};
    use crate::{Error, address::Protocol, transport::TlsRoots};

    #[test]
    fn fills_every_session_configuration_field() {
        let builder = Builder::new("irc.example.net")
            .nick("ping-bot")
            .alt_nicks(["ping-bot_", "ping-bot__"])
            .username("ping")
            .realname("papo ping bot")
            .password("network password")
            .sasl(Sasl::plain("ping-bot", "correct horse battery staple"))
            .require_sasl(true)
            .device("phone")
            .caps(["message-tags", "sasl"]);
        let config = &builder.config;
        assert_eq!(config.nick, "ping-bot");
        assert_eq!(config.alt_nicks, ["ping-bot_", "ping-bot__"]);
        assert_eq!(config.username, "ping");
        assert_eq!(config.realname, "papo ping bot");
        assert_eq!(config.password.as_deref(), Some("network password"));
        assert!(config.sasl.is_some());
        assert!(config.require_sasl);
        assert_eq!(config.device.as_deref(), Some("phone"));
        assert_eq!(config.wanted_caps, ["message-tags", "sasl"]);
    }

    #[test]
    fn nick_supplies_only_implicit_username_and_realname_defaults() {
        let implicit = Builder::new("irc.example.net").nick("ping-bot").config;
        assert_eq!(
            (implicit.username.as_str(), implicit.realname.as_str()),
            ("ping-bot", "ping-bot")
        );
        let explicit = Builder::new("irc.example.net")
            .username("service")
            .realname("Papo service")
            .nick("ping-bot")
            .config;
        assert_eq!(
            (explicit.username.as_str(), explicit.realname.as_str()),
            ("service", "Papo service")
        );
    }

    #[test]
    fn configures_timing_rate_and_event_capacity() {
        let builder = Builder::new("irc.example.net")
            .rate_limit(RateLimit::unlimited())
            .ping_interval(Duration::from_secs(31))
            .ping_timeout(Duration::from_secs(93))
            .event_capacity(17);
        assert_eq!(builder.config.rate_limit, RateLimit::unlimited());
        assert_eq!(builder.config.ping_interval, Duration::from_secs(31));
        assert_eq!(builder.config.ping_timeout, Duration::from_secs(93));
        assert_eq!(builder.event_capacity, 17);
    }

    #[test]
    #[should_panic(expected = "event capacity must be positive")]
    fn rejects_an_empty_event_queue() {
        _ = Builder::new("irc.example.net").event_capacity(0);
    }

    #[test]
    fn configures_ports_and_security() {
        let plain = Builder::new("irc.example.net").plaintext();
        let address = plain.address.unwrap();
        assert_eq!((address.protocol, address.port), (Protocol::Irc, 6667));

        let websocket = Builder::new("ws://[2001:db8::1]/webirc")
            .port(8443)
            .tls(true);
        let address = websocket.address.unwrap();
        assert_eq!((address.protocol, address.port), (Protocol::Wss, 8443));
        let explicit = Builder::new("irc.example.net:7000")
            .plaintext()
            .address
            .unwrap();
        assert_eq!(explicit.port, 7000);
    }

    #[test]
    fn plaintext_keeps_an_explicit_tls_default_port() {
        let address = Builder::new("irc.example.net:6697")
            .plaintext()
            .address
            .unwrap();
        assert_eq!((address.protocol, address.port), (Protocol::Irc, 6697));
    }

    #[test]
    fn address_security_reaches_the_session_configuration() {
        for (input, plaintext, expected) in [
            ("ircs://irc.example.net", false, ConnectionSecurity::Secure),
            ("irc.example.net", false, ConnectionSecurity::Secure),
            (
                "irc://irc.example.net",
                false,
                ConnectionSecurity::Plaintext,
            ),
            ("irc.example.net", true, ConnectionSecurity::Plaintext),
            ("wss://irc.example.net", false, ConnectionSecurity::Secure),
            ("ws://irc.example.net", false, ConnectionSecurity::Plaintext),
        ] {
            let mut builder = Builder::new(input);
            if plaintext {
                builder = builder.plaintext();
            }
            let address = builder.address.as_ref().unwrap();
            assert_eq!(address.is_secure(), expected == ConnectionSecurity::Secure);
            apply_address(&mut builder.config, address);
            assert_eq!(builder.config.connection_security, expected);
        }
    }

    #[test]
    fn configures_tls_material_and_names_file_errors() {
        let builder = Builder::new("irc.example.net")
            .tls_ca_pem(b"test CA".to_vec())
            .tls_client_cert(b"certificate".to_vec(), b"private key".to_vec());
        assert!(matches!(builder.tls.roots, TlsRoots::Ca(pem) if pem == b"test CA"));
        assert!(builder.tls.client_certificate.is_some());

        let error = Builder::new("irc.example.net")
            .tls_ca_file("missing-papo-builder-ca.pem")
            .err()
            .unwrap();
        assert!(
            matches!(&error, Error::ReadFile { path, .. } if path == std::path::Path::new("missing-papo-builder-ca.pem"))
        );
        assert!(error.to_string().contains("missing-papo-builder-ca.pem"));
    }

    #[test]
    fn configures_post_registration_defaults() {
        let builder = Builder::new("irc.example.net")
            .bot()
            .channels(["#papo", "#papo-bots"]);
        assert!(builder.bot);
        assert_eq!(builder.channels, ["#papo", "#papo-bots"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn build_defers_address_errors_until_the_driver_is_polled() {
        let (_client, mut events, driver) = Builder::new("irc://").build();
        assert!(matches!(driver.await, Err(Error::Address(_))));
        assert!(events.next().await.is_none());
    }

    #[cfg(feature = "spawn")]
    #[tokio::test(flavor = "current_thread")]
    async fn connect_returns_a_deferred_address_error() {
        let error = Builder::new("irc://").connect().await.err().unwrap();
        assert!(matches!(error, Error::Address(_)));
    }

    #[cfg(not(feature = "websocket"))]
    #[tokio::test(flavor = "current_thread")]
    async fn websocket_without_feature_is_typed_as_unsupported() {
        let (_client, _events, driver) = Builder::new("ws://irc.example.net/webirc")
            .reconnect(crate::Reconnect::never())
            .build();
        assert!(matches!(driver.await, Err(Error::Unsupported("websocket"))));
    }
}
