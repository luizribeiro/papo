use std::{fmt, time::Duration};

use papo_core::sts::{ConnectionSecurity, MemoryStsStore, StsStore};
use papo_core::{
    AwayOnConnect, Config, CtcpReplies, DEFAULT_WANTED_CAPS, RateLimit, Sasl, WhoOnJoin, cap,
};
use papo_proto::{Line, escape_tag_value};
use tokio::sync::oneshot;

#[cfg(not(browser))]
use crate::transport::{ClientCertificate, TlsRoots};
use crate::{
    Error, InvalidInput, NewAccount, Reconnect,
    address::ServerAddress,
    driver::{Autojoin, ConnectionPlan, PostConnect},
    transport::TlsConfig,
};

/// Configures an IRC client before it is built or connected.
#[must_use]
#[allow(clippy::struct_excessive_bools)]
pub struct Builder {
    pub(crate) address: Result<ServerAddress, InvalidInput>,
    pub(crate) config: Config,
    pub(crate) event_capacity: usize,
    pub(crate) tls: TlsConfig,
    connect_timeout: Duration,
    reconnect: Reconnect,
    sts_store: Box<dyn StsStore + Send>,
    #[cfg(not(browser))]
    stream: Option<Box<dyn crate::io_compat::FutureStream>>,
    bot: bool,
    autojoin: Vec<Autojoin>,
    history_on_reconnect: bool,
    username_set: bool,
    realname_set: bool,
    invalid_input: Option<InvalidInput>,
}

impl fmt::Debug for Builder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("Builder");
        debug
            .field("address", &self.address)
            .field("config", &self.config)
            .field("event_capacity", &self.event_capacity)
            .field("tls", &self.tls)
            .field("connect_timeout", &self.connect_timeout)
            .field("reconnect", &self.reconnect)
            .field("sts_store", &"<STS store>");
        #[cfg(not(browser))]
        debug.field("stream", &self.stream.as_ref().map(|_| "<stream>"));
        debug
            .field("bot", &self.bot)
            .field("autojoin", &self.autojoin)
            .field("history_on_reconnect", &self.history_on_reconnect)
            .field("invalid_input", &self.invalid_input)
            .finish_non_exhaustive()
    }
}

impl Builder {
    pub(crate) fn new(address: &str) -> Self {
        let mut config = Config::default();
        config.ctcp.version = Some(format!("papo {}", crate::VERSION));
        Self {
            address: address.parse(),
            config,
            event_capacity: crate::driver::DEFAULT_CHANNEL_CAPACITY,
            tls: TlsConfig::default(),
            connect_timeout: Duration::from_secs(30),
            reconnect: Reconnect::default(),
            sts_store: Box::new(MemoryStsStore::new()),
            #[cfg(not(browser))]
            stream: None,
            bot: false,
            autojoin: Vec::new(),
            history_on_reconnect: false,
            username_set: false,
            realname_set: false,
            invalid_input: None,
        }
    }

    pub(crate) fn from_endpoint(endpoint: impl crate::IntoEndpoint) -> Self {
        let mut builder = Self::new("irc.example.net");
        builder.address = crate::address::convert(endpoint);
        builder
    }

    /// Seeds the server-defined name comparison rules used before connecting.
    ///
    /// A server advertisement replaces these rules for the active connection. The most recently
    /// advertised rules remain available while disconnected.
    pub const fn casemapping(mut self, casemapping: crate::Casemapping) -> Self {
        self.config.casemapping = casemapping;
        self
    }

    /// Sets the primary nickname.
    pub fn nick(mut self, nick: impl AsRef<str>) -> Self {
        let nick = nick.as_ref().to_owned();
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
        S: AsRef<str>,
    {
        self.config.alt_nicks = nicks
            .into_iter()
            .map(|nick| nick.as_ref().to_owned())
            .collect();
        self
    }

    /// Chooses whether to reclaim the primary nickname after a connection-time collision.
    ///
    /// This is disabled by default. The client uses `MONITOR` when advertised and otherwise
    /// polls one `ISON` request at a time.
    pub const fn regain_nick(mut self, regain: bool) -> Self {
        self.config.regain_nick = regain;
        self
    }

    /// Sets the username sent while connecting.
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.config.username = username.into();
        self.username_set = true;
        self
    }

    /// Sets the real name sent while connecting.
    pub fn realname(mut self, realname: impl Into<String>) -> Self {
        self.config.realname = realname.into();
        self.realname_set = true;
        self
    }

    /// Sets the server password sent with `PASS`.
    pub fn password(mut self, password: impl Into<papo_core::Secret>) -> Self {
        let password = password.into();
        self.reject_binary_secret(&password);
        self.config.password = Some(password);
        self
    }

    /// Sets the identity asserted through a trusted `WEBIRC` gateway.
    pub fn webirc(mut self, webirc: crate::WebIrc) -> Self {
        if webirc.password.expose().is_none() {
            self.invalid_input.get_or_insert(InvalidInput::BinarySecret);
        }
        self.config = self.config.webirc(webirc.into_core());
        self
    }

    /// Configures SASL authentication.
    pub fn sasl(mut self, sasl: Sasl) -> Self {
        if sasl.has_binary_secret() {
            self.invalid_input.get_or_insert(InvalidInput::BinarySecret);
        }
        self.config.sasl = Some(sasl);
        self
    }

    /// Selects an upstream bouncer network before the connection completes.
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub fn bouncer_network(mut self, id: crate::NetworkId) -> Self {
        self.config = self.config.enable(crate::Feature::Bouncer);
        self.config.bouncer_network = Some(id);
        self
    }

    /// Enables an unstable extension feature for capability negotiation.
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub fn enable(mut self, feature: crate::Feature) -> Self {
        self.config = self.config.enable(feature);
        self
    }

    /// Appends `@<device>` to the SASL username for PLAIN and SCRAM.
    ///
    /// Bouncers and servers can use the suffix to distinguish a user's clients. This setting has
    /// no effect with EXTERNAL or OAUTHBEARER.
    pub fn device(mut self, device: impl Into<String>) -> Self {
        self.config.device = Some(device.into());
        self
    }

    /// Creates an account while connecting.
    ///
    /// ```
    /// let account = papo::NewAccount::new("correct horse battery staple")
    ///     .account("alice");
    /// # _ = papo::Client::builder("ircs://irc.example.net").create_account(account);
    /// ```
    pub fn create_account(mut self, account: NewAccount) -> Self {
        self.reject_binary_secret(&account.password);
        if !self
            .config
            .wanted_caps
            .iter()
            .any(|name| cap::canonical(name) == cap::canonical(cap::ACCOUNT_REGISTRATION.as_str()))
        {
            self.config
                .wanted_caps
                .push(cap::ACCOUNT_REGISTRATION.into());
        }
        self.config =
            self.config
                .create_account_on_connect(account.account, account.email, account.password);
        self
    }

    /// Adds capability names to request when advertised.
    ///
    /// ```
    /// let builder = papo::Client::builder("ircs://irc.example.net")
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
    /// ```
    /// let builder = papo::Client::builder("ircs://irc.example.net")
    ///     .without_caps([papo::cap::AWAY_NOTIFY]);
    /// # _ = builder;
    /// ```
    pub fn without_caps<I, S>(mut self, caps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for capability in caps.into_iter().map(Into::into) {
            let canonical = cap::canonical(&capability).to_owned();
            if !DEFAULT_WANTED_CAPS
                .iter()
                .any(|name| cap::canonical(name) == canonical)
                && !cap::is_unstable(&capability)
            {
                self.invalid_input
                    .get_or_insert(InvalidInput::UnknownCapability(capability));
                continue;
            }
            self.config = self.config.without_caps([capability]);
            self.config
                .wanted_caps
                .retain(|name| cap::canonical(name) != canonical);
        }
        self
    }

    /// Sets metadata keys subscribed on every connection.
    pub fn metadata_subscriptions<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<crate::MetadataKey>,
    {
        self.config = self.config.metadata_subscriptions(
            keys.into_iter()
                .map(Into::into)
                .map(crate::MetadataKey::into_string),
        );
        self
    }

    /// Sets the outbound line rate limit.
    ///
    /// The limit is ignored while the server advertises `SAFERATE` unless
    /// [`RateLimit::strict`] was selected.
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

    /// Sets how long an action waits for its server response after all of its
    /// lines have been sent.
    ///
    /// A timeout means no reply arrived in time, not that the server did
    /// nothing. Retrying an action such as `KICK` or `MODE +b` can repeat its
    /// effect.
    ///
    /// ```
    /// # use std::time::Duration;
    /// # _ = papo::Client::builder("ircs://irc.example.net")
    /// #     .request_timeout(Duration::from_secs(15));
    /// ```
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.config.request_timeout = timeout;
        self
    }

    /// Sets the away message announced while connecting.
    ///
    /// ```
    /// # _ = papo::Client::builder("ircs://irc.example.net").away_on_connect("At lunch");
    /// ```
    pub fn away_on_connect(mut self, text: impl Into<String>) -> Self {
        self.config.away_on_connect = Some(AwayOnConnect::Message(text.into()));
        self
    }

    /// Sets the automatic CTCP reply policy.
    ///
    /// Channel queries get a private reply.
    ///
    /// ```
    /// # use papo::CtcpReplies;
    /// # _ = papo::Client::builder("ircs://irc.example.net")
    /// #     .ctcp_replies(CtcpReplies::default().version(Some("papo bot")));
    /// ```
    pub fn ctcp_replies(mut self, replies: CtcpReplies) -> Self {
        self.config.ctcp = replies;
        self
    }

    /// Sets the automatic reply to CTCP SOURCE queries.
    ///
    /// Channel queries get a private reply.
    ///
    /// ```
    /// # _ = papo::Client::builder("ircs://irc.example.net")
    /// #     .ctcp_source("https://example.org/papo-bot");
    /// ```
    pub fn ctcp_source(mut self, url: impl Into<String>) -> Self {
        self.config.ctcp.source = Some(url.into());
        self
    }

    /// Sets the bounded event queue capacity.
    ///
    /// When both this queue and its equally sized overflow fill, the connection
    /// ends with [`DisconnectCause::SlowConsumer`](crate::DisconnectCause::SlowConsumer)
    /// and follows the reconnect policy. Move slow work out of the event loop
    /// before raising this bound.
    ///
    pub fn event_capacity(mut self, capacity: usize) -> Self {
        if capacity == 0 {
            self.invalid_input
                .get_or_insert(InvalidInput::ZeroEventCapacity);
        }
        self.event_capacity = capacity;
        self
    }

    /// Sets the policy for reconnecting after an unexpected disconnect.
    pub const fn reconnect(mut self, reconnect: Reconnect) -> Self {
        self.reconnect = reconnect;
        self
    }

    /// Sets the deadline for opening each TCP, TLS, or WebSocket connection.
    ///
    /// The default is 30 seconds. The deadline applies to every attempt,
    /// including the first attempt made by [`Builder::connect`].
    pub const fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Sets the store used for IRC Strict Transport Security policies.
    #[cfg(not(browser))]
    pub fn sts_store(mut self, store: impl StsStore + Send + 'static) -> Self {
        self.sts_store = Box::new(store);
        self
    }

    /// Uses an already-connected asynchronous byte stream.
    ///
    /// Custom streams are driven once and do not reconnect or apply STS policies.
    #[cfg(not(browser))]
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
    /// # _ = papo::Client::builder("ircs://irc.example.net").tls_ca_pem(Vec::new());
    /// ```
    #[cfg(not(browser))]
    pub fn tls_ca_pem(mut self, pem: impl Into<Vec<u8>>) -> Self {
        self.tls.roots = TlsRoots::Ca(pem.into());
        self
    }

    /// Adds certificates from a PEM document to the platform trust roots.
    ///
    /// ```
    /// # _ = papo::Client::builder("ircs://irc.example.net").tls_extra_ca_pem(Vec::new());
    /// ```
    #[cfg(not(browser))]
    pub fn tls_extra_ca_pem(mut self, pem: impl Into<Vec<u8>>) -> Self {
        self.tls.roots = TlsRoots::PlatformAndCa(pem.into());
        self
    }

    /// Sets a PEM-encoded TLS client certificate chain and private key.
    ///
    /// ```no_run
    /// # fn configure(cert: &std::path::Path, key: &std::path::Path) -> std::io::Result<papo::Builder> {
    /// use std::fs;
    /// let builder = papo::Client::builder("ircs://irc.example.net")
    ///     .tls_client_cert(fs::read(cert)?, fs::read(key)?);
    /// # Ok(builder)
    /// # }
    /// ```
    #[cfg(not(browser))]
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
    /// # _ = papo::Client::builder("ircs://127.0.0.1").tls_server_name("irc.example.net");
    /// ```
    #[cfg(not(browser))]
    pub fn tls_server_name(mut self, server_name: impl Into<String>) -> Self {
        self.tls.server_name = Some(server_name.into());
        self
    }

    /// Sets whether the server-advertised bot user mode is enabled after login.
    ///
    /// The default is `false`. When enabled, this is skipped if the server offers no bot mode.
    /// After connecting, [`crate::State::supports`] with [`crate::Feature::BotMode`] reports
    /// whether the server offers one.
    pub fn bot(mut self, enabled: bool) -> Self {
        self.bot = enabled;
        self
    }

    /// Adds channels to join after connecting.
    ///
    /// Repeated exact names are replaced by the last entry. Once the server's casemapping is
    /// known, names that compare equal under that mapping are also deduplicated before joining.
    ///
    /// ```
    /// # _ = papo::Client::builder("ircs://irc.example.net").autojoin(["#papo"]);
    /// ```
    pub fn autojoin<I, S>(mut self, channels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for channel in channels {
            self.push_autojoin(Autojoin::new(channel.as_ref(), None));
        }
        self
    }

    /// Adds a keyed channel to join after connecting.
    ///
    /// This replaces an exact configured name. Once the server's casemapping is known, names
    /// that compare equal under that mapping are also deduplicated before joining.
    ///
    /// ```
    /// # _ = papo::Client::builder("ircs://irc.example.net").autojoin_with_key("#staff", "secret");
    /// ```
    pub fn autojoin_with_key(
        mut self,
        channel: impl AsRef<str>,
        key: impl Into<papo_core::Secret>,
    ) -> Self {
        let key = key.into();
        self.reject_binary_secret(&key);
        self.push_autojoin(Autojoin::new(channel.as_ref(), Some(key)));
        self
    }

    /// Sets when channel members are refreshed with `WHO` after joining.
    ///
    /// The default is [`crate::WhoOnJoin::Always`]. A refresh happens once after the initial
    /// `NAMES` reply; later member joins do not start another query.
    pub fn who_on_join(mut self, policy: WhoOnJoin) -> Self {
        self.config.who_on_join = policy;
        self
    }

    fn reject_binary_secret(&mut self, secret: &papo_core::Secret) {
        if secret.expose().is_none() {
            self.invalid_input.get_or_insert(InvalidInput::BinarySecret);
        }
    }

    fn push_autojoin(&mut self, entry: Autojoin) {
        self.autojoin
            .retain(|configured| configured.channel != entry.channel);
        self.autojoin.push(entry);
    }

    /// Sets whether network reconnects fetch history missed while disconnected.
    ///
    /// This emits one history event per channel joined during this process. It does not recover
    /// direct messages, channels from an earlier process, or in-memory history lost when the IRC
    /// server itself restarts. Full clients should use [`crate::Client::recent_conversations`] and
    /// [`crate::Client::history`] with [`crate::History::after`] and a persisted cursor.
    pub const fn history_on_reconnect(mut self, enabled: bool) -> Self {
        self.history_on_reconnect = enabled;
        self
    }

    /// Builds handles and a lazy driver without opening a connection.
    ///
    /// The returned [`crate::Driver`] connects when it is first polled.
    pub fn build(self) -> (crate::Client, crate::Events, crate::Driver) {
        self.build_with_connect(None, None)
    }

    /// Connects in a spawned driver and waits until the connection is ready.
    ///
    /// To observe [`crate::Event::Connecting`] during the first attempt, use
    /// [`Builder::build`] and poll its event stream while running the returned driver. This method
    /// returns the event stream only after that attempt is ready.
    ///
    /// The [`crate::Event::Connected`] event remains available from the
    /// returned event stream. When `draft/account-registration` requires email
    /// verification, this returns after [`crate::AccountCreation::VerificationRequired`]
    /// because the server holds the connection open until verification.
    /// The first connection attempt fails fast; the configured reconnect policy applies only
    /// after the first successful connection.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for deferred endpoint or builder validation,
    /// [`Error::Connect`] when the transport cannot open or IRC login fails before welcome or an
    /// account-verification challenge.
    pub async fn connect(self) -> Result<(crate::Client, crate::Events), Error> {
        let (sender, receiver) = oneshot::channel();
        let (client, events, driver) = self.build_with_connect(Some(sender), None);
        crate::rt::spawn(driver);
        receiver.await.map_err(|_| Error::connection_lost())??;
        Ok((client, events))
    }

    #[cfg(not(browser))]
    fn build_stream_with_connect(
        mut self,
        stream: Box<dyn crate::io_compat::FutureStream>,
        registration: Option<crate::driver::ConnectSender>,
    ) -> (crate::Client, crate::Events, crate::Driver) {
        if let Ok(address) = &self.address {
            apply_address(&mut self.config, address);
        }
        let input_error = self
            .address
            .err()
            .or(self.invalid_input)
            .or_else(|| connect_input_error(&self.config));
        let session =
            papo_core::Session::with_entropy(self.config, Box::new(crate::entropy::OsEntropy));
        if let Some(error) = input_error {
            drop(stream);
            return crate::Driver::failed(
                &session,
                self.event_capacity.max(1),
                registration,
                Error::InvalidInput(error),
            );
        }
        crate::Driver::with_stream_and_connect(
            crate::io_compat::FuturesIo::new(stream),
            session,
            self.event_capacity,
            registration,
            PostConnect {
                bot: self.bot,
                channels: self.autojoin,
                parted_while_disconnected: Vec::new(),
                history_on_reconnect: self.history_on_reconnect,
                disconnected_at: None,
            },
        )
    }

    pub(crate) fn build_with_connector(
        self,
        connector: crate::driver::Connector,
    ) -> (crate::Client, crate::Events, crate::Driver) {
        self.build_with_connect(None, Some(connector))
    }

    fn build_with_connect(
        mut self,
        registration: Option<crate::driver::ConnectSender>,
        connector: Option<crate::driver::Connector>,
    ) -> (crate::Client, crate::Events, crate::Driver) {
        #[cfg(not(browser))]
        if let Some(stream) = self.stream.take() {
            return self.build_stream_with_connect(stream, registration);
        }
        let validation_error = self
            .invalid_input
            .or_else(|| connect_input_error(&self.config));
        let plan = self.address.and_then(|address| {
            if let Some(error) = validation_error {
                return Err(error);
            }
            #[cfg(tls)]
            if address.is_secure() {
                crate::transport::validate_tls_config(&address.host, &self.tls)
                    .map_err(InvalidInput::TlsConfig)?;
            }
            apply_address(&mut self.config, &address);
            Ok(ConnectionPlan {
                address,
                tls: self.tls,
            })
        });
        let session =
            papo_core::Session::with_entropy(self.config, Box::new(crate::entropy::OsEntropy));
        let defaults = PostConnect {
            bot: self.bot,
            channels: self.autojoin,
            parted_while_disconnected: Vec::new(),
            history_on_reconnect: self.history_on_reconnect,
            disconnected_at: None,
        };
        crate::Driver::with_plan(
            plan,
            session,
            self.event_capacity.max(1),
            registration,
            defaults,
            self.reconnect,
            self.connect_timeout,
            self.sts_store,
            connector,
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

fn connect_input_error(config: &Config) -> Option<InvalidInput> {
    let invalid = |message: Line| {
        message
            .to_line()
            .err()
            .map(|error| InvalidInput::InvalidMessage(error.to_string()))
    };
    if let Some(error) = invalid(Line::new("NICK", [&config.nick]))
        .or_else(|| {
            invalid(Line::new(
                "USER",
                [&config.username, "0", "*", &config.realname],
            ))
        })
        .or_else(|| {
            config.password.as_ref().and_then(|password| {
                password
                    .expose()
                    .and_then(|password| invalid(Line::new("PASS", [password])))
            })
        })
        .or_else(|| {
            config
                .alt_nicks
                .iter()
                .find_map(|nick| invalid(Line::new("NICK", [nick])))
        })
        .or_else(|| match &config.away_on_connect {
            Some(AwayOnConnect::Message(message)) => invalid(Line::new("AWAY", [message])),
            Some(_) | None => None,
        })
    {
        return Some(error);
    }
    let webirc = config.webirc.as_ref()?;
    let password = webirc.password.expose()?;
    let client_ip = webirc.client_ip.to_string();
    let ip = if client_ip.starts_with(':') {
        format!("0{client_ip}")
    } else {
        client_ip
    };
    let hostname = webirc.client_hostname.as_deref().unwrap_or(&ip);
    let options = webirc
        .options
        .iter()
        .map(|(name, value)| match value {
            Some(value) => format!("{name}={}", escape_tag_value(value)),
            None => name.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut params = vec![password, &webirc.gateway, hostname, &ip];
    if !options.is_empty() {
        params.push(&options);
    }
    invalid(Line::new("WEBIRC", params))
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::{
        io,
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll},
        time::Duration,
    };

    use papo_core::{
        CtcpReplies, DEFAULT_WANTED_CAPS, RateLimit, Sasl, Secret, sts::ConnectionSecurity,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{Builder, apply_address, connect_input_error};
    use crate::{
        ConnectErrorKind, Error, InvalidInput, NewAccount,
        address::Protocol,
        driver::test_support::{
            abort_task, client_line, scripted, server_line, welcome_burst, within,
        },
        transport::TlsRoots,
    };

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn connect_timeout_covers_a_silent_tls_peer() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let connection = tokio::spawn(
            Builder::new(&format!("ircs://{address}"))
                .connect_timeout(Duration::from_secs(17))
                .connect(),
        );

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(16)).await;
        assert!(!connection.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(matches!(
            connection.await.unwrap(),
            Err(Error::Connect(error)) if error.kind() == ConnectErrorKind::TimedOut
        ));
        peer.abort();
    }

    macro_rules! disconnected {
        () => {
            Error::Disconnected(_)
        };
    }

    struct ObservedStream {
        inner: crate::io_compat::TokioIo<tokio::io::DuplexStream>,
        lifecycle: Arc<AtomicUsize>,
    }

    impl futures_io::AsyncRead for ObservedStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.inner).poll_read(context, buffer)
        }
    }

    impl futures_io::AsyncWrite for ObservedStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.inner).poll_write(context, buffer)
        }

        fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.lifecycle.store(1, Ordering::Relaxed);
            Pin::new(&mut self.inner).poll_flush(context)
        }

        fn poll_close(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
            assert_eq!(self.lifecycle.load(Ordering::Relaxed), 1);
            self.lifecycle.store(2, Ordering::Relaxed);
            Pin::new(&mut self.inner).poll_close(context)
        }
    }

    #[test]
    fn fills_every_session_configuration_field() {
        let builder = Builder::new("irc.example.net")
            .casemapping(crate::Casemapping::Ascii)
            .nick(crate::Casemapping::default().nick("ping-bot"))
            .alt_nicks([
                crate::Casemapping::default().nick("ping-bot_"),
                crate::Casemapping::default().nick("ping-bot__"),
            ])
            .regain_nick(true)
            .username("ping")
            .realname("papo ping bot")
            .password("network password")
            .sasl(Sasl::plain("ping-bot", "correct horse battery staple"))
            .device("phone")
            .request_caps(["message-tags", "sasl"]);
        let config = &builder.config;
        assert_eq!(config.casemapping, crate::Casemapping::Ascii);
        assert_eq!(config.nick, "ping-bot");
        assert_eq!(config.alt_nicks, ["ping-bot_", "ping-bot__"]);
        assert!(config.regain_nick);
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

    #[tokio::test(flavor = "current_thread")]
    async fn configured_nickname_regain_reaches_the_wire() {
        let mut builder = Builder::new("irc.example.net")
            .nick("papo-driver")
            .username("papo")
            .realname("papo driver test")
            .alt_nicks(["papo-driver_"])
            .regain_nick(true);
        builder.config.wanted_caps.clear();
        let (mut server, _client, mut events, driver) = scripted(builder.config);
        let driver = tokio::spawn(driver);

        welcome_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test 433 * papo-driver :Nickname is already in use\r\n",
        )
        .await;
        client_line(&mut server, b"NICK papo-driver_\r\n").await;
        server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(&mut server, b":ergo.test 001 papo-driver_ :Welcome to ErgoTest\r\n:ergo.test 005 papo-driver_ MONITOR=100 :are supported\r\n:ergo.test 422 papo-driver_ :MOTD File is missing\r\n").await;
        assert!(matches!(
            events.next().await,
            Some(crate::Event::Connected(_))
        ));
        client_line(&mut server, b"MONITOR + papo-driver\r\n").await;
        server_line(&mut server, b":ergo.test 731 papo-driver_ :papo-driver\r\n").await;
        client_line(&mut server, b"NICK papo-driver\r\n").await;
        assert!(matches!(
            events.next().await,
            Some(crate::Event::MonitorOffline(_))
        ));
        abort_task(driver).await;
    }

    #[test]
    fn seeds_offline_state_name_rules() {
        let (client, _events, _driver) = Builder::new("irc.example.net")
            .casemapping(crate::Casemapping::Ascii)
            .nick(crate::Casemapping::Ascii.nick("papo[bot]"))
            .build();

        let state = client.state();
        assert!(!state.is_connected());
        assert_eq!(state.casemapping(), crate::Casemapping::Ascii);
        assert_eq!(state.nick().casemapping(), crate::Casemapping::Ascii);
        assert_ne!(state.nick(), "papo{bot}");
    }

    #[test]
    fn account_registration_adds_or_retains_one_capability() {
        let added = Builder::new("irc.example.net")
            .without_caps(["draft/account-registration"])
            .create_account(NewAccount::new("realistic secret"))
            .config;
        assert_eq!(
            added
                .wanted_caps
                .iter()
                .filter(|name| name.as_str() == "draft/account-registration")
                .count(),
            1
        );

        let added = Builder::new("irc.example.net")
            .request_caps(["message-tags"])
            .create_account(NewAccount::new("realistic secret"))
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
            .create_account(NewAccount::new("realistic secret"))
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
            .request_caps(["message-tags", "example.com/custom-cap"])
            .without_caps(["away-notify"]);
        assert!(
            !builder
                .config
                .wanted_caps
                .iter()
                .any(|name| name == "away-notify")
        );
        assert!(
            builder
                .config
                .wanted_caps
                .iter()
                .any(|name| name == "example.com/custom-cap")
        );
        let aliases = Builder::new("irc.example.net")
            .without_caps(["read-marker", "account-registration"])
            .config;
        for removed in ["draft/read-marker", "draft/account-registration"] {
            assert!(!aliases.wanted_caps.iter().any(|name| name == removed));
        }
        let unstable_caps = [
            "soju.im/bouncer-networks",
            "soju.im/bouncer-networks-notify",
            "draft/persistence",
            "draft/relaymsg",
            "draft/webpush",
            "soju.im/webpush",
        ];
        for capability in unstable_caps {
            let unstable = Builder::new("irc.example.net").without_caps([capability]);
            assert_eq!(unstable.invalid_input, None);
            assert_eq!(unstable.config.disabled_caps, [capability]);
        }
    }

    #[cfg(feature = "unstable")]
    #[test]
    fn enables_unstable_features_and_bouncer_selection_before_connecting() {
        let enabled = Builder::new("irc.example.net")
            .enable(crate::Feature::Persistence)
            .enable(crate::Feature::Relay)
            .enable(crate::Feature::WebPush)
            .bouncer_network(crate::NetworkId::from("libera"));
        assert!(
            enabled
                .config
                .enabled_features
                .contains(&crate::Feature::Bouncer)
        );
        for capability in [
            "soju.im/bouncer-networks",
            "soju.im/bouncer-networks-notify",
            "draft/persistence",
            "draft/relaymsg",
            "draft/webpush",
        ] {
            assert!(
                enabled
                    .config
                    .wanted_caps
                    .iter()
                    .any(|name| name == capability)
            );
        }
    }

    #[test]
    fn nick_supplies_only_implicit_username_and_realname_defaults() {
        let implicit = Builder::new("irc.example.net")
            .nick(crate::Casemapping::default().nick("ping-bot"))
            .config;
        assert_eq!(
            (implicit.username.as_str(), implicit.realname.as_str()),
            ("ping-bot", "ping-bot")
        );
        let explicit = Builder::new("irc.example.net")
            .username("service")
            .realname("Papo service")
            .nick(crate::Casemapping::default().nick("ping-bot"))
            .config;
        assert_eq!(
            (explicit.username.as_str(), explicit.realname.as_str()),
            ("service", "Papo service")
        );
    }

    #[test]
    fn configures_timing_rate_and_event_capacity() {
        assert_eq!(
            Builder::new("irc.example.net").config.request_timeout,
            Duration::from_secs(30)
        );
        let builder = Builder::new("irc.example.net")
            .rate_limit(RateLimit::unlimited())
            .ping_interval(Duration::from_secs(31))
            .ping_timeout(Duration::from_secs(93))
            .request_timeout(Duration::from_secs(17))
            .event_capacity(17);
        assert_eq!(builder.config.rate_limit, RateLimit::unlimited());
        assert_eq!(builder.config.ping_interval, Duration::from_secs(31));
        assert_eq!(builder.config.ping_timeout, Duration::from_secs(93));
        assert_eq!(builder.config.request_timeout, Duration::from_secs(17));
        assert_eq!(builder.event_capacity, 17);
    }

    #[test]
    fn configures_away_and_ctcp_replies() {
        let rate = RateLimit::new(4, Duration::from_secs(3)).unwrap();
        assert_eq!(
            Builder::new("irc.example.net")
                .config
                .ctcp
                .version
                .as_deref(),
            Some(concat!("papo ", env!("CARGO_PKG_VERSION")))
        );
        let builder = Builder::new("irc.example.net")
            .away_on_connect("Reviewing patches")
            .ctcp_replies(CtcpReplies::default().version(None).ping(false).rate(rate))
            .ctcp_source("https://example.org/papo");
        assert_eq!(
            builder.config.away_on_connect,
            Some(papo_core::AwayOnConnect::Message(
                "Reviewing patches".into()
            ))
        );
        assert_eq!(builder.config.ctcp.version, None);
        assert!(!builder.config.ctcp.ping);
        assert_eq!(builder.config.ctcp.rate, rate);
        assert_eq!(
            builder.config.ctcp.source.as_deref(),
            Some("https://example.org/papo")
        );
    }

    #[test]
    fn debug_redacts_every_builder_secret() {
        let builder = Builder::new("irc.example.net")
            .password("unique-network-password-9134")
            .webirc(crate::WebIrc {
                password: "unique-webirc-password-3186".into(),
                gateway: "papo-gateway".into(),
                client_ip: "192.0.2.42".parse().unwrap(),
                client_hostname: None,
                options: Vec::new(),
            })
            .sasl(Sasl::plain("alice", "unique-sasl-password-8527"))
            .create_account(NewAccount::new("unique-account-password-7416"))
            .tls_client_cert(Vec::new(), "unique-tls-key-6305")
            .autojoin_with_key("#staff", "unique-channel-key-5294");
        let output = format!("{builder:?}");
        assert!(output.contains("[REDACTED]"));
        for secret in [
            "unique-network-password-9134",
            "unique-webirc-password-3186",
            "unique-sasl-password-8527",
            "unique-account-password-7416",
            "unique-tls-key-6305",
            "unique-channel-key-5294",
        ] {
            assert!(!output.contains(secret));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn defers_invalid_setters_until_build() {
        let cases = [(
            Builder::new("irc.example.net").event_capacity(0),
            InvalidInput::ZeroEventCapacity,
        )];
        for (builder, expected) in cases {
            let (_client, _events, driver) = builder.build();
            assert!(matches!(
                within(driver).await,
                Err(Error::InvalidInput(actual)) if actual == expected
            ));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn defers_binary_text_secrets_until_build() {
        let byte_secret = || Secret::from_bytes([0xff]);
        let cases = [
            Builder::new("irc.example.net").password(byte_secret()),
            Builder::new("irc.example.net").webirc(crate::WebIrc {
                password: byte_secret(),
                gateway: "papo-gateway".into(),
                client_ip: "192.0.2.42".parse().unwrap(),
                client_hostname: None,
                options: Vec::new(),
            }),
            Builder::new("irc.example.net").sasl(Sasl::plain("alice", byte_secret())),
            Builder::new("irc.example.net").create_account(
                NewAccount::new(byte_secret())
                    .account(crate::Casemapping::default().account("alice")),
            ),
            Builder::new("irc.example.net").autojoin_with_key("#staff", byte_secret()),
        ];
        for builder in cases {
            let (_client, _events, driver) = builder.build();
            assert!(matches!(
                within(driver).await,
                Err(Error::InvalidInput(InvalidInput::BinarySecret))
            ));
        }

        assert!(
            Builder::new("irc.example.net")
                .tls_client_cert(Vec::new(), byte_secret())
                .invalid_input
                .is_none()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_invalid_registration_messages_before_connecting() {
        let cases = [
            Builder::new("irc.example.net")
                .nick(crate::Casemapping::default().nick("papo\rOPER root password")),
            Builder::new("irc.example.net")
                .alt_nicks([crate::Casemapping::default().nick("papo-away\nQUIT")]),
            Builder::new("irc.example.net").username("papo\0admin"),
            Builder::new("irc.example.net").realname("Papo\rClient"),
            Builder::new("irc.example.net").password("network\npassword"),
            Builder::new("irc.example.net").webirc(crate::WebIrc {
                password: "gateway-password".into(),
                gateway: "papo\rgateway".into(),
                client_ip: "192.0.2.42".parse().unwrap(),
                client_hostname: None,
                options: Vec::new(),
            }),
            Builder::new("irc.example.net").away_on_connect("gone\nAWAY"),
            Builder::new("irc.example.net")
                .username("u".repeat(250))
                .realname("r".repeat(251)),
        ];
        for builder in cases {
            let (_client, _events, driver) = builder.build();
            assert!(matches!(
                within(driver).await,
                Err(Error::InvalidInput(InvalidInput::InvalidMessage(_)))
            ));
        }
    }

    #[test]
    fn accepts_webirc_without_options() {
        let config = Builder::new("irc.example.net")
            .webirc(crate::WebIrc {
                password: "gateway-password".into(),
                gateway: "papo-gateway".into(),
                client_ip: "192.0.2.42".parse().unwrap(),
                client_hostname: None,
                options: Vec::new(),
            })
            .config;
        assert_eq!(connect_input_error(&config), None);
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
        assert_eq!(
            Builder::new("irc.example.net").config.who_on_join,
            crate::WhoOnJoin::Always
        );
        let staff = crate::Casemapping::default().channel("#staff");
        assert!(!Builder::new("irc.example.net").bot(false).bot);
        let builder = Builder::new("irc.example.net")
            .bot(true)
            .autojoin(["#papo", "#papo-bots"])
            .autojoin_with_key(staff, "private channel key")
            .who_on_join(crate::WhoOnJoin::BelowMembers(25))
            .history_on_reconnect(true);
        assert!(builder.bot);
        assert_eq!(
            builder
                .autojoin
                .iter()
                .map(|entry| entry.channel.as_str())
                .collect::<Vec<_>>(),
            ["#papo", "#papo-bots", "#staff"]
        );
        assert!(builder.autojoin[2].key.is_some());
        assert_eq!(
            builder.config.who_on_join,
            crate::WhoOnJoin::BelowMembers(25)
        );
        assert!(builder.history_on_reconnect);
    }

    #[test]
    fn autojoin_replaces_only_exact_names_before_registration() {
        let builder = Builder::new("irc.example.net")
            .autojoin(["#papo", "#papo", "#PAPO"])
            .autojoin_with_key("#papo", "private channel key");
        assert_eq!(
            builder
                .autojoin
                .iter()
                .map(|entry| entry.channel.as_str())
                .collect::<Vec<_>>(),
            ["#PAPO", "#papo"]
        );
        assert!(builder.autojoin[1].key.is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn build_defers_address_errors_until_the_driver_is_polled() {
        let (_client, mut events, driver) = Builder::new("irc://").build();
        assert!(matches!(within(driver).await, Err(Error::InvalidInput(_))));
        assert!(events.next().await.is_none());
    }

    #[cfg(tls)]
    #[tokio::test(flavor = "current_thread")]
    async fn build_rejects_invalid_tls_input_before_connecting() {
        let (_client, _events, driver) =
            crate::Client::builder(crate::Endpoint::tls("localhost", 9).unwrap())
                .tls_ca_pem(Vec::new())
                .reconnect(crate::Reconnect::never())
                .build();
        assert!(matches!(
            within(driver).await,
            Err(Error::InvalidInput(InvalidInput::TlsConfig(message)))
                if message.contains("no certificates")
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn public_builder_rejects_malformed_string_endpoints() {
        for input in [
            "",
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
    fn public_builder_accepts_owned_and_borrowed_server_addresses() {
        for (builder, protocol, port) in [
            (
                crate::Client::builder("irc.example.net"),
                Protocol::Ircs,
                6697,
            ),
            (
                crate::Client::builder(String::from("irc://irc.example.net:7000")),
                Protocol::Irc,
                7000,
            ),
        ] {
            let address = builder.address.unwrap();
            assert_eq!((address.protocol, address.port), (protocol, port));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connect_returns_a_deferred_address_error() {
        let error = within(Builder::new("irc://").connect())
            .await
            .err()
            .unwrap();
        assert!(matches!(error, Error::InvalidInput(_)));
        let error = within(Builder::new("irc.example.net").event_capacity(0).connect())
            .await
            .err()
            .unwrap();
        assert_eq!(error.to_string(), "event queue capacity must be positive");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connect_classifies_a_peer_close_before_welcome() {
        let (stream, server) = tokio::io::duplex(4096);
        drop(server);
        let error = within(
            Builder::new("irc://custom.example")
                .stream(crate::io_compat::TokioIo(stream))
                .connect(),
        )
        .await
        .err()
        .expect("an early peer close must fail connect");
        let Error::Connect(error) = error else {
            panic!("early close was not a connect error: {error:?}")
        };
        assert_eq!(error.kind(), ConnectErrorKind::Other);
        assert_eq!(
            std::error::Error::source(&error).map(ToString::to_string),
            Some("broken pipe".into())
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn custom_stream_registers_once_without_reconnecting() {
        let (stream, mut server) = tokio::io::duplex(4096);
        let lifecycle = Arc::new(AtomicUsize::new(0));
        let connecting = tokio::spawn(
            Builder::new("irc://custom.example")
                .nick(crate::Casemapping::default().nick("papo-custom"))
                .stream(ObservedStream {
                    inner: crate::io_compat::TokioIo(stream),
                    lifecycle: Arc::clone(&lifecycle),
                })
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
        assert!(client.state().is_connected());
        assert_eq!(client.state().nick(), "papo-custom");
        assert!(matches!(
            events.next().await,
            Some(crate::Event::Connected(_))
        ));
        assert!(matches!(client.reconnect().await, Err(disconnected!())));
        let quitting = tokio::spawn({
            let client = client.clone();
            async move { client.quit("").await }
        });
        let mut quit = [0; 6];
        within(server.read_exact(&mut quit)).await.unwrap();
        assert_eq!(&quit, b"QUIT\r\n");
        within(server.shutdown()).await.unwrap();
        assert!(within(quitting).await.unwrap().is_ok());
        assert!(matches!(
            events.next().await,
            Some(crate::Event::Disconnected(disconnected)) if !disconnected.is_retrying()
        ));
        assert!(!client.state().is_connected());
        assert_eq!(lifecycle.load(Ordering::Relaxed), 2);
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
            Err(Error::Connect(error)) if error.kind() == crate::ConnectErrorKind::Unavailable
        ));
    }
}
