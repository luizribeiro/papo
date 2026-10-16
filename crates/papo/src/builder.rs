use std::time::Duration;

use papo_core::sts::{ConnectionSecurity, MemoryStsStore, StsStore};
use papo_core::{Config, DEFAULT_WANTED_CAPS, RateLimit, Sasl};
use tokio::sync::oneshot;

use crate::{
    Error, InvalidInput, Reconnect,
    address::ServerAddress,
    driver::{ConnectionPlan, PostRegistration},
    transport::{ClientCertificate, TlsConfig, TlsRoots},
};

/// Configures an IRC client before it is built or connected.
#[must_use]
#[allow(clippy::struct_excessive_bools)]
pub struct Builder {
    pub(crate) address: Result<ServerAddress, InvalidInput>,
    pub(crate) config: Config,
    pub(crate) event_capacity: usize,
    pub(crate) tls: TlsConfig,
    reconnect: Reconnect,
    disconnected_queue_capacity: usize,
    sts_store: Box<dyn StsStore + Send>,
    stream: Option<Box<dyn crate::io_compat::FutureStream>>,
    bot: bool,
    channels: Vec<String>,
    history_on_reconnect: bool,
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
            sts_store: Box::new(MemoryStsStore::new()),
            stream: None,
            bot: false,
            channels: Vec::new(),
            history_on_reconnect: false,
            username_set: false,
            realname_set: false,
        }
    }

    pub(crate) fn from_endpoint(endpoint: impl crate::IntoEndpoint) -> Self {
        let mut builder = Self::new("irc.example.net");
        builder.address = crate::address::convert(endpoint).map(crate::Endpoint::into_address);
        builder
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
    pub fn password(mut self, password: impl Into<papo_core::Secret>) -> Self {
        self.config.password = Some(password.into());
        self
    }

    /// Sets the identity asserted through a trusted `WEBIRC` gateway.
    pub fn webirc(mut self, webirc: papo_core::WebIrc) -> Self {
        self.config = self.config.webirc(webirc);
        self
    }

    /// Configures SASL authentication.
    pub fn sasl(mut self, sasl: Sasl) -> Self {
        self.config.sasl = Some(sasl);
        self
    }

    /// Sets the Ergo multiclient device identifier.
    pub fn device(mut self, device: impl Into<String>) -> Self {
        self.config.device = Some(device.into());
        self
    }

    /// Registers an account before connection registration completes.
    pub fn register_on_connect(
        mut self,
        account: Option<impl Into<String>>,
        email: Option<impl Into<String>>,
        password: impl Into<papo_core::Secret>,
    ) -> Self {
        if !self.config.wanted_caps.iter().any(|capability| {
            matches!(
                capability.as_str(),
                "account-registration" | "draft/account-registration"
            )
        }) {
            self.config
                .wanted_caps
                .push("draft/account-registration".into());
        }
        self.config = self.config.register_on_connect(account, email, password);
        self
    }

    /// Adds capability names to request when advertised.
    ///
    /// ```
    /// let builder = papo::Client::builder("irc.example.net")
    ///     .request_caps(["soju.im/bouncer-networks"]);
    /// # _ = builder;
    /// ```
    pub fn request_caps<I, S>(mut self, caps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for capability in caps.into_iter().map(Into::into) {
            if !self.config.wanted_caps.contains(&capability) {
                self.config.wanted_caps.push(capability);
            }
        }
        self
    }

    /// Removes default capability names from the request set.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidInput::UnknownCapability`] for a name papo does not request by default.
    ///
    /// ```
    /// # fn main() -> Result<(), papo::InvalidInput> {
    /// let builder = papo::Client::builder("irc.example.net")
    ///     .without_caps([papo::cap::AWAY_NOTIFY])?;
    /// # _ = builder;
    /// # Ok(())
    /// # }
    /// ```
    pub fn without_caps<I, S>(mut self, caps: I) -> Result<Self, InvalidInput>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for capability in caps.into_iter().map(Into::into) {
            if !DEFAULT_WANTED_CAPS.contains(&capability.as_str()) {
                return Err(InvalidInput::UnknownCapability(capability));
            }
            self.config.wanted_caps.retain(|name| name != &capability);
        }
        Ok(self)
    }

    /// Sets metadata keys subscribed on every connection.
    pub fn metadata_subscriptions<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config = self.config.metadata_subscriptions(keys);
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

    /// Sets the store used for IRC Strict Transport Security policies.
    pub fn sts_store(mut self, store: impl StsStore + Send + 'static) -> Self {
        self.sts_store = Box::new(store);
        self
    }

    /// Uses an already-connected asynchronous byte stream.
    ///
    /// Custom streams are driven once and do not reconnect or apply STS policies.
    pub fn stream(
        mut self,
        stream: impl futures_io::AsyncRead + futures_io::AsyncWrite + Send + Unpin + 'static,
    ) -> Self {
        self.stream = Some(Box::new(stream));
        self
    }

    /// Trusts only the certificates in a PEM document.
    ///
    /// ```
    /// # _ = papo::Client::builder("irc.example.net").tls_ca_pem(Vec::new());
    /// ```
    pub fn tls_ca_pem(mut self, pem: impl Into<Vec<u8>>) -> Self {
        self.tls.roots = TlsRoots::Ca(pem.into());
        self
    }

    /// Adds certificates from a PEM document to the platform trust roots.
    ///
    /// ```
    /// # _ = papo::Client::builder("irc.example.net").tls_extra_ca_pem(Vec::new());
    /// ```
    pub fn tls_extra_ca_pem(mut self, pem: impl Into<Vec<u8>>) -> Self {
        self.tls.roots = TlsRoots::PlatformAndCa(pem.into());
        self
    }

    /// Sets a PEM-encoded TLS client certificate chain and private key.
    ///
    /// ```
    /// # _ = papo::Client::builder("irc.example.net").tls_client_cert(Vec::new(), "key");
    /// ```
    pub fn tls_client_cert(
        mut self,
        certificate: impl Into<Vec<u8>>,
        key: impl Into<papo_core::Secret>,
    ) -> Self {
        self.tls.client_certificate = Some(ClientCertificate::new(certificate, key));
        self
    }

    /// Overrides the DNS name authenticated by TLS.
    ///
    /// ```
    /// # _ = papo::Client::builder("127.0.0.1").tls_server_name("irc.example.net");
    /// ```
    pub fn tls_server_name(mut self, server_name: impl Into<String>) -> Self {
        self.tls.server_name = Some(server_name.into());
        self
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

    /// Sets whether reconnects fetch history missed while disconnected.
    pub const fn history_on_reconnect(mut self, enabled: bool) -> Self {
        self.history_on_reconnect = enabled;
        self
    }

    /// Builds handles and a lazy driver without opening a connection.
    ///
    /// The returned [`crate::Driver`] connects when it is first polled.
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
    pub async fn connect(self) -> Result<(crate::Client, crate::Events), Error> {
        let (sender, receiver) = oneshot::channel();
        let (client, events, driver) = self.build_with_registration(Some(sender));
        crate::rt::spawn(driver);
        receiver.await.map_err(|_| Error::Disconnected)??;
        Ok((client, events))
    }

    fn build_stream_with_registration(
        mut self,
        stream: Box<dyn crate::io_compat::FutureStream>,
        registration: Option<crate::driver::RegistrationSender>,
    ) -> (crate::Client, crate::Events, crate::Driver) {
        if let Ok(address) = &self.address {
            apply_address(&mut self.config, address);
        }
        let address_error = self.address.err();
        let session = papo_core::Session::with_entropy(self.config, Box::new(crate::OsEntropy));
        if let Some(error) = address_error {
            drop(stream);
            return crate::Driver::failed(
                &session,
                self.event_capacity,
                registration,
                Error::InvalidInput(error),
            );
        }
        crate::Driver::with_stream_and_registration(
            crate::io_compat::FuturesIo::new(stream),
            session,
            self.event_capacity,
            registration,
            PostRegistration {
                bot: self.bot,
                channels: self.channels,
                history_on_reconnect: self.history_on_reconnect,
            },
        )
    }

    fn build_with_registration(
        mut self,
        registration: Option<crate::driver::RegistrationSender>,
    ) -> (crate::Client, crate::Events, crate::Driver) {
        if let Some(stream) = self.stream.take() {
            return self.build_stream_with_registration(stream, registration);
        }
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
            history_on_reconnect: self.history_on_reconnect,
        };
        crate::Driver::with_plan(
            plan,
            session,
            self.event_capacity,
            registration,
            defaults,
            self.reconnect,
            self.disconnected_queue_capacity,
            self.sts_store,
        )
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

    use papo_core::{DEFAULT_WANTED_CAPS, RateLimit, Sasl, Secret, sts::ConnectionSecurity};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::{Builder, apply_address};
    use crate::{
        Error, InvalidInput, address::Protocol, driver::test_support::within, transport::TlsRoots,
    };

    #[test]
    fn fills_every_session_configuration_field() {
        let builder = Builder::new("irc.example.net")
            .nick("ping-bot")
            .alt_nicks(["ping-bot_", "ping-bot__"])
            .username("ping")
            .realname("papo ping bot")
            .password("network password")
            .sasl(Sasl::plain("ping-bot", "correct horse battery staple"))
            .device("phone")
            .request_caps(["message-tags", "sasl"]);
        let config = &builder.config;
        assert_eq!(config.nick, "ping-bot");
        assert_eq!(config.alt_nicks, ["ping-bot_", "ping-bot__"]);
        assert_eq!(config.username, "ping");
        assert_eq!(config.realname, "papo ping bot");
        assert_eq!(config.password, Some(Secret::new("network password")));
        assert!(config.sasl.is_some());
        assert_eq!(config.device.as_deref(), Some("phone"));
        assert_eq!(
            config.wanted_caps[..DEFAULT_WANTED_CAPS.len()],
            DEFAULT_WANTED_CAPS[..]
        );
        assert_eq!(config.wanted_caps.last().map(String::as_str), Some("sasl"));
    }

    #[test]
    fn account_registration_retains_one_default_capability() {
        let added = Builder::new("irc.example.net")
            .request_caps(["message-tags"])
            .register_on_connect(None::<String>, None::<String>, "realistic secret")
            .config;
        assert_eq!(
            added
                .wanted_caps
                .iter()
                .filter(|name| name.as_str() == "draft/account-registration")
                .count(),
            1
        );

        let retained = Builder::new("irc.example.net")
            .request_caps(["server-time", "account-registration"])
            .register_on_connect(None::<String>, None::<String>, "realistic secret")
            .config;
        assert!(
            retained
                .wanted_caps
                .iter()
                .any(|name| name == "account-registration")
        );
    }

    #[test]
    fn capabilities_extend_defaults_and_only_known_defaults_can_be_removed() {
        let builder = Builder::new("irc.example.net")
            .request_caps(["message-tags", "soju.im/bouncer-networks"])
            .without_caps(["away-notify"])
            .unwrap();
        assert!(
            !builder
                .config
                .wanted_caps
                .iter()
                .any(|name| name == "away-notify")
        );
        assert_eq!(
            builder.config.wanted_caps.last().map(String::as_str),
            Some("soju.im/bouncer-networks")
        );
        assert!(matches!(
            Builder::new("irc.example.net").without_caps(["soju.im/bouncer-networks"]),
            Err(InvalidInput::UnknownCapability(name)) if name == "soju.im/bouncer-networks"
        ));
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
    fn address_security_reaches_the_session_configuration() {
        for (input, expected) in [
            ("ircs://irc.example.net", ConnectionSecurity::Secure),
            ("irc.example.net", ConnectionSecurity::Secure),
            ("irc://irc.example.net", ConnectionSecurity::Plaintext),
            ("wss://irc.example.net", ConnectionSecurity::Secure),
            ("ws://irc.example.net", ConnectionSecurity::Plaintext),
        ] {
            let mut builder = Builder::new(input);
            let address = builder.address.as_ref().unwrap();
            assert_eq!(address.is_secure(), expected == ConnectionSecurity::Secure);
            apply_address(&mut builder.config, address);
            assert_eq!(builder.config.connection_security, expected);
        }
    }

    #[test]
    fn configures_tls_material_and_server_name() {
        let builder = Builder::new("irc.example.net")
            .tls_extra_ca_pem(b"test CA".to_vec())
            .tls_client_cert(b"certificate".to_vec(), "private key")
            .tls_server_name("chat.example.net");
        assert!(matches!(builder.tls.roots, TlsRoots::PlatformAndCa(pem) if pem == b"test CA"));
        assert!(builder.tls.client_certificate.is_some());
        assert_eq!(builder.tls.server_name.as_deref(), Some("chat.example.net"));
    }

    #[test]
    fn configures_post_registration_defaults() {
        let builder = Builder::new("irc.example.net")
            .bot()
            .channels(["#papo", "#papo-bots"])
            .history_on_reconnect(true);
        assert!(builder.bot);
        assert_eq!(builder.channels, ["#papo", "#papo-bots"]);
        assert!(builder.history_on_reconnect);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn build_defers_address_errors_until_the_driver_is_polled() {
        let (_client, mut events, driver) = Builder::new("irc://").build();
        assert!(matches!(within(driver).await, Err(Error::InvalidInput(_))));
        assert!(events.next().await.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn public_builder_rejects_every_non_hostname_string() {
        for input in [
            "",
            "irc://irc.example.net",
            "irc.example.net:6697",
            "irc.example.net/webirc",
            "alice@irc.example.net",
            "irc example.net",
            "irc.example.net\t",
        ] {
            let (_client, _events, driver) = crate::Client::builder(input).build();
            assert!(
                matches!(within(driver).await, Err(Error::InvalidInput(_))),
                "accepted {input:?}"
            );
        }
    }

    #[test]
    fn public_builder_accepts_owned_and_borrowed_hostnames() {
        for builder in [
            crate::Client::builder("irc.example.net"),
            crate::Client::builder(String::from("irc.example.net")),
        ] {
            let address = builder.address.unwrap();
            assert_eq!((address.protocol, address.port), (Protocol::Ircs, 6697));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connect_returns_a_deferred_address_error() {
        let error = within(Builder::new("irc://").connect())
            .await
            .err()
            .unwrap();
        assert!(matches!(error, Error::InvalidInput(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn custom_stream_registers_once_without_reconnecting() {
        let (stream, mut server) = tokio::io::duplex(4096);
        let connecting = tokio::spawn(
            Builder::new("irc://custom.example")
                .nick("papo-custom")
                .stream(crate::io_compat::TokioIo(stream))
                .connect(),
        );
        let expected = b"CAP LS 302\r\nNICK papo-custom\r\nUSER papo-custom 0 * papo-custom\r\n";
        let mut burst = vec![0; expected.len()];
        within(server.read_exact(&mut burst)).await.unwrap();
        assert_eq!(&burst, expected);
        within(server.write_all(b":custom.example CAP * LS :\r\n"))
            .await
            .unwrap();
        let mut cap_end = [0; 9];
        within(server.read_exact(&mut cap_end)).await.unwrap();
        assert_eq!(&cap_end, b"CAP END\r\n");
        within(server.write_all(
            b":custom.example 001 papo-custom :Welcome\r\n:custom.example 422 papo-custom :MOTD File is missing\r\n",
        ))
        .await
        .unwrap();
        let (client, mut events) = within(connecting).await.unwrap().unwrap();
        assert!(client.is_connected());
        assert!(client.state(|state| state.connected() && state.nick() == "papo-custom"));
        assert!(matches!(
            events.next().await,
            Some(crate::Event::Registered(_))
        ));
        assert!(matches!(client.reconnect().await, Err(Error::Disconnected)));
        within(server.shutdown()).await.unwrap();
        assert!(matches!(
            events.next().await,
            Some(crate::Event::Disconnected {
                will_retry: false,
                ..
            })
        ));
        assert!(!client.is_connected());
        assert!(matches!(client.quit(None).await, Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn custom_stream_build_defers_address_errors() {
        let (stream, _server) = tokio::io::duplex(64);
        let (_client, mut events, driver) = Builder::new("irc://")
            .stream(crate::io_compat::TokioIo(stream))
            .build();
        assert!(matches!(within(driver).await, Err(Error::InvalidInput(_))));
        assert!(events.next().await.is_none());
    }

    #[cfg(not(feature = "websocket"))]
    #[tokio::test(flavor = "current_thread")]
    async fn websocket_without_feature_is_typed_as_unsupported() {
        let (_client, _events, driver) = Builder::new("ws://irc.example.net/webirc")
            .reconnect(crate::Reconnect::never())
            .build();
        assert!(matches!(
            within(driver).await,
            Err(Error::Unsupported("websocket"))
        ));
    }
}
