use std::{
    fmt,
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use papo_core::sts::{ConnectionSecurity, MemoryStsStore, StsStore};
use papo_core::{AwayOnConnect, Config, CtcpReplies, RateLimit, Sasl, WhoOnJoin, cap};
use tokio::sync::oneshot;

#[cfg(not(browser))]
use crate::transport::{ClientCertificate, TlsRoots};
use crate::{
    Error, InvalidInput, NewAccount, Reconnect,
    address::ServerAddress,
    driver::{Autojoin, ConnectionPlan, PostConnect},
    transport::TlsConfig,
};

static NEXT_ORIGIN: AtomicU64 = AtomicU64::new(1);

/// Declares whether a caller-provided stream encrypts its bytes.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum StreamSecurity {
    /// The stream carries plaintext IRC traffic.
    #[default]
    Plaintext,
    /// The stream is protected by encryption established by the caller.
    Encrypted,
}

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct NetworkBuilderSettings {
    pub(crate) address: Result<ServerAddress, InvalidInput>,
    config: Config,
    tls: TlsConfig,
    connect_timeout: Duration,
    reconnect: Reconnect,
    sts_store: Arc<dyn StsStore>,
    credentials_configured: bool,
    allow_insecure_auth: bool,
    reusable_transport: bool,
}

fn allocate_origin() -> crate::Origin {
    let mut value = NEXT_ORIGIN.load(Ordering::Relaxed);
    loop {
        let next = value.checked_add(1).expect("client origin space exhausted");
        match NEXT_ORIGIN.compare_exchange_weak(value, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return crate::Origin::__new(value),
            Err(actual) => value = actual,
        }
    }
}

/// Configures an IRC client before it is built or connected.
///
/// Settings that require a capability, including account creation, metadata subscriptions,
/// reconnect history, and selecting a bouncer network, fail connection setup when that
/// capability is unavailable. [`Self::enable`] opts into known optional behavior only when its
/// capabilities are offered. [`Self::request_caps`] is for vendor capabilities unknown to papo.
#[must_use]
#[allow(clippy::struct_excessive_bools)]
pub struct Builder {
    pub(crate) address: Result<ServerAddress, InvalidInput>,
    pub(crate) config: Config,
    pub(crate) event_capacity: usize,
    pub(crate) tls: TlsConfig,
    connect_timeout: Duration,
    reconnect: Reconnect,
    sts_store: Arc<dyn StsStore>,
    #[cfg(not(browser))]
    stream: Option<(Box<dyn crate::io_compat::FutureStream>, StreamSecurity)>,
    bot: bool,
    autojoin: Vec<Autojoin>,
    history_on_reconnect: bool,
    username_set: bool,
    realname_set: bool,
    invalid_input: Option<InvalidInput>,
    credentials_configured: bool,
    allow_insecure_auth: bool,
}

impl fmt::Debug for Builder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        crate::opaque_debug("Builder", formatter)
    }
}

impl Builder {
    pub(crate) fn new(address: &str) -> Self {
        let mut config = Config::default();
        config.ctcp = config.ctcp.version(format!("papo {}", crate::VERSION));
        Self {
            address: address.parse().map_err(InvalidInput::from),
            config,
            event_capacity: crate::driver::DEFAULT_CHANNEL_CAPACITY,
            tls: TlsConfig::default(),
            connect_timeout: Duration::from_secs(30),
            reconnect: Reconnect::default(),
            sts_store: Arc::new(MemoryStsStore::new()),
            #[cfg(not(browser))]
            stream: None,
            bot: false,
            autojoin: Vec::new(),
            history_on_reconnect: false,
            username_set: false,
            realname_set: false,
            invalid_input: None,
            credentials_configured: false,
            allow_insecure_auth: false,
        }
    }

    pub(crate) fn from_endpoint(
        endpoint: impl crate::IntoEndpoint,
        nick: impl Into<String>,
    ) -> Self {
        let mut builder = Self::new("ircs://irc.example.net");
        builder.address = crate::address::convert(endpoint);
        let nick = nick.into();
        builder.config.nick.clone_from(&nick);
        builder.config.username.clone_from(&nick);
        builder.config.realname = nick;
        builder
    }

    #[allow(dead_code)]
    pub(crate) fn from_network_settings(settings: &NetworkBuilderSettings) -> Self {
        let mut builder = Self::new("ircs://irc.example.net");
        builder.address.clone_from(&settings.address);
        builder.config = settings.config.clone();
        builder.tls = settings.tls.clone();
        builder.connect_timeout = settings.connect_timeout;
        builder.reconnect = settings.reconnect;
        builder.sts_store = Arc::clone(&settings.sts_store);
        builder.credentials_configured = settings.credentials_configured;
        builder.allow_insecure_auth = settings.allow_insecure_auth;
        if !settings.reusable_transport {
            builder.invalid_input = Some(InvalidInput::CustomStreamNotReusable);
        }
        builder
    }

    pub(crate) fn network_settings(&self, reusable_transport: bool) -> NetworkBuilderSettings {
        NetworkBuilderSettings {
            address: self.address.clone(),
            config: self.config.__additional_connection(),
            tls: self.tls.clone(),
            connect_timeout: self.connect_timeout,
            reconnect: self.reconnect,
            sts_store: Arc::clone(&self.sts_store),
            credentials_configured: self.credentials_configured,
            allow_insecure_auth: self.allow_insecure_auth,
            reusable_transport,
        }
    }

    /// Seeds the server-defined name comparison rules used before connecting.
    ///
    /// This makes [`crate::State::name_rules`] available before the first `005` response.
    ///
    /// A server advertisement replaces these rules for the active connection. The most recently
    /// advertised rules remain available while disconnected.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() {
    /// let builder = papo::Client::builder("irc://offline.test", "papo-doc")
    ///     .casemapping(papo::Casemapping::Ascii);
    /// # let (client, mut events, driver, mut connections) =
    /// #     papo::__testing::wire_client(builder);
    /// let before = client.state();
    /// let rules = before.name_rules().expect("casemapping was seeded");
    /// assert_eq!(rules.check_nick("release_candidate"), papo::NameCheck::Unknown);
    /// # let driver = tokio::spawn(driver);
    /// # let (mut server, _) = connections.recv().await.unwrap();
    /// # papo::__testing::server_line(&mut server, b":offline.test CAP * LS :\r\n").await;
    /// # papo::__testing::server_line(&mut server, b":offline.test 005 papo-doc CASEMAPPING=ascii NICKLEN=12 :are supported\r\n:offline.test 001 papo-doc :Welcome\r\n:offline.test 422 papo-doc :MOTD is missing\r\n").await;
    /// # while !matches!(events.next().await, Some(papo::Event::Connected(_))) {}
    /// let after = client.state();
    /// let rules = after.name_rules().expect("005 supplied name rules");
    /// assert!(matches!(rules.check_nick("release_candidate"),
    ///     papo::NameCheck::Invalid(_)));
    /// # driver.abort();
    /// # }
    /// ```
    pub fn casemapping(mut self, casemapping: crate::Casemapping) -> Self {
        self.config = self.config.casemapping(casemapping);
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
    ///
    /// On a plaintext endpoint, the server must offer a valid `sts` upgrade before the password
    /// is sent. [`Self::allow_insecure_auth`] explicitly permits sending it without that upgrade.
    pub fn password(mut self, password: impl Into<papo_core::Secret>) -> Self {
        self.config.password = Some(password.into());
        self
    }

    /// Sets the identity asserted through a trusted `WEBIRC` gateway.
    pub fn webirc(mut self, webirc: crate::WebIrc) -> Self {
        self.config = self.config.webirc(webirc.into_core());
        self.credentials_configured = true;
        self
    }

    /// Configures required SASL authentication.
    ///
    /// The connection fails with [`ConnectErrorKind::SaslUnavailable`](crate::ConnectErrorKind::SaslUnavailable)
    /// when the server offers no usable SASL capability or mechanism.
    pub fn sasl(mut self, sasl: Sasl) -> Self {
        self.config.sasl = Some(sasl);
        self.credentials_configured = true;
        self
    }

    /// Selects an upstream bouncer network before the connection completes.
    ///
    /// The connection fails unless the server acknowledges `soju.im/bouncer-networks` and its
    /// welcome confirms this exact ID through `BOUNCER_NETID`. Bound connections do not request
    /// network-change notifications by default; call [`Self::enable`] with
    /// [`Feature::Bouncer`](crate::Feature::Bouncer) to opt in. Enabling before or after selecting
    /// the network has the same effect.
    #[cfg(feature = "unstable")]
    pub fn bouncer_network(mut self, id: crate::NetworkId) -> Self {
        if !self
            .config
            .wanted_caps
            .iter()
            .any(|name| cap::canonical(name) == cap::canonical(cap::BOUNCER_NETWORKS.as_str()))
        {
            self.config
                .wanted_caps
                .push(cap::BOUNCER_NETWORKS.as_str().to_owned());
        }
        self.config.bouncer_network = Some(id);
        self
    }

    /// Enables an optional feature for capability negotiation.
    ///
    /// Stable features are already on by default. Calling this after [`Self::disable`] turns the
    /// feature back on; the last call wins.
    pub fn enable(mut self, feature: crate::Feature) -> Self {
        self.config = self.config.enable(feature);
        self
    }

    /// Disables an optional feature for capability negotiation.
    ///
    /// Calling [`Self::enable`] afterward turns the feature back on; the last call wins.
    pub fn disable(mut self, feature: crate::Feature) -> Self {
        self.config = self.config.disable(feature);
        self
    }

    /// Creates an account while connecting.
    ///
    /// This requires `draft/account-registration` with `before-connect`; a named account that
    /// differs from the nick also requires `custom-account-name`. Otherwise the connection fails
    /// with [`ConnectErrorKind::CapabilityUnavailable`](crate::ConnectErrorKind::CapabilityUnavailable).
    ///
    /// ```
    /// let account = papo::NewAccount::new(papo::Secret::new("correct horse battery staple"))
    ///     .account("alice");
    /// # _ = papo::Client::builder("ircs://irc.example.net", "papo-client").create_account(account);
    /// ```
    pub fn create_account(mut self, account: NewAccount) -> Self {
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
        self.config = self.config.enable(crate::Feature::AccountCreation);
        self.config =
            self.config
                .create_account_on_connect(account.account, account.email, account.password);
        self.credentials_configured = true;
        self
    }

    /// Allows configured credentials on an unencrypted `irc://` or `ws://` endpoint.
    ///
    /// SASL, WEBIRC, and account registration require `ircs://` or `wss://` by default.
    /// Credentials are refused on plaintext endpoints unless the transport is protected by
    /// another trusted layer, such as a local test fixture. A server password on plaintext TCP
    /// is sent only after a valid IRC STS upgrade; enabling this also permits the password without
    /// that upgrade.
    pub fn allow_insecure_auth(mut self, allow: bool) -> Self {
        self.allow_insecure_auth = allow;
        self.config = self.config.allow_insecure_auth(allow);
        self
    }

    /// Adds capability names unknown to papo to request when advertised.
    ///
    /// ```
    /// let builder = papo::Client::builder("ircs://irc.example.net", "papo-client")
    ///     .request_caps(["vendor.example/network-state"]);
    /// # _ = builder;
    /// ```
    pub fn request_caps<I, S>(mut self, caps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for capability in caps.into_iter().map(Into::into) {
            if !cfg!(feature = "unstable") && cap::is_unstable(&capability) {
                self.invalid_input
                    .get_or_insert(InvalidInput::UnstableCapability(capability));
                continue;
            }
            if let Some(feature) = cap::__feature(&capability) {
                self.invalid_input
                    .get_or_insert(InvalidInput::KnownCapability {
                        capability,
                        feature: feature.__variant_name(),
                    });
                continue;
            }
            if cap::is_known(&capability) {
                self.invalid_input
                    .get_or_insert(InvalidInput::ManagedCapability(capability));
                continue;
            }
            if !self.config.wanted_caps.contains(&capability) {
                self.config.wanted_caps.push(capability);
            }
        }
        self
    }

    /// Sets metadata keys subscribed on every connection.
    ///
    /// A nonempty subscription requires `draft/metadata-3`; otherwise the connection fails with
    /// [`ConnectErrorKind::CapabilityUnavailable`](crate::ConnectErrorKind::CapabilityUnavailable).
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
    ///
    /// [`Duration::MAX`] disables keepalive probes.
    pub fn ping_interval(mut self, interval: Duration) -> Self {
        self.config.ping_interval = interval;
        self
    }

    /// Sets the maximum time allowed without an incoming line.
    ///
    /// [`Duration::MAX`] disables the silence deadline.
    pub fn ping_timeout(mut self, timeout: Duration) -> Self {
        self.config.ping_timeout = timeout;
        self
    }

    /// Sets how long an action waits for its server response after all of its
    /// lines have been sent. [`Duration::MAX`] disables this deadline.
    ///
    /// A timeout means no reply arrived in time, not that the server did
    /// nothing. Retrying an action such as `KICK` or `MODE +b` can repeat its
    /// effect.
    ///
    /// ```
    /// # use std::time::Duration;
    /// # _ = papo::Client::builder("ircs://irc.example.net", "papo-client")
    /// #     .request_timeout(Duration::from_secs(15));
    /// ```
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.config.request_timeout = timeout;
        self
    }

    /// Sets the away message announced while connecting.
    ///
    /// ```
    /// # _ = papo::Client::builder("ircs://irc.example.net", "papo-client").away_on_connect("At lunch");
    /// ```
    pub fn away_on_connect(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        if text.is_empty() {
            self.invalid_input.get_or_insert(InvalidInput::EmptyText);
        } else {
            self.config.away_on_connect = Some(AwayOnConnect::Message(text));
        }
        self
    }

    /// Sets the automatic CTCP reply policy.
    ///
    /// See [`CtcpReplies`] for the default replies and the fixed live, private-target safety
    /// rules.
    ///
    /// ```
    /// # use papo::CtcpReplies;
    /// # _ = papo::Client::builder("ircs://irc.example.net", "papo-client")
    /// #     .ctcp_replies(CtcpReplies::default().version("papo bot"));
    /// ```
    pub fn ctcp_replies(mut self, replies: CtcpReplies) -> Self {
        self.config.ctcp = replies;
        self
    }

    /// Sets the bounded event queue capacity.
    ///
    /// When this queue fills, papo keeps reading the connection and reports discarded events in
    /// [`Event::Lagged`](crate::Event::Lagged). Changes already reflected in state—membership,
    /// modes, topics, presence, metadata, read markers, and typing—are discarded first, followed
    /// by other non-message events, and then messages, notices, CTCP, tag messages, reactions, and
    /// redactions. One lag event reports each lag episode. Re-read
    /// [`Client::state`](crate::Client::state) for state-backed changes and use
    /// [`Client::history`](crate::Client::history) for reported message gaps. Gap filling is not
    /// automatic.
    ///
    /// The default is 1,024 events. Size this for the largest burst the event loop may
    /// legitimately pause through, not the average event rate: a client receiving 250 events per
    /// second whose event loop can pause for two seconds needs at least 500 slots, plus headroom.
    /// Move slow work out of that loop before raising the bound. A larger value consumes more
    /// memory during bursts and delays the warning; it does not prevent lag indefinitely. Values
    /// above the runtime's supported bound are clamped to that bound, and zero is invalid.
    pub fn event_capacity(mut self, capacity: usize) -> Self {
        if capacity == 0 {
            self.invalid_input
                .get_or_insert(InvalidInput::ZeroEventCapacity);
        }
        self.event_capacity = capacity.min(tokio::sync::Semaphore::MAX_PERMITS);
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
    /// including the first attempt made by [`Builder::connect`]. [`Duration::MAX`]
    /// disables this deadline.
    pub const fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Sets the store used for IRC Strict Transport Security policies.
    #[cfg(not(browser))]
    pub fn sts_store(mut self, store: impl StsStore + 'static) -> Self {
        self.sts_store = Arc::new(store);
        self
    }

    /// Trusts only the certificates in a PEM document.
    ///
    /// This and [`Self::tls_extra_ca_pem`] replace the previously configured roots; the last call
    /// wins.
    ///
    /// An IP endpoint can use a private CA while authenticating the DNS name in the certificate:
    ///
    /// ```no_run
    /// # async fn connect() -> Result<(), Box<dyn std::error::Error>> {
    /// use std::fs;
    ///
    /// let ca_pem = fs::read("private-ca.pem")?;
    /// let (_client, _events) = papo::Client::builder("ircs://127.0.0.1:6697", "papo-client")
    ///     .tls_ca_pem(ca_pem)
    ///     .tls_server_name("irc.internal.example")
    ///     .connect()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(browser))]
    pub fn tls_ca_pem(mut self, pem: impl Into<Vec<u8>>) -> Self {
        self.tls.roots = TlsRoots::Ca(pem.into());
        self
    }

    /// Adds certificates from a PEM document to the platform trust roots.
    ///
    /// This and [`Self::tls_ca_pem`] replace the previously configured roots; the last call wins.
    ///
    /// ```
    /// # _ = papo::Client::builder("ircs://irc.example.net", "papo-client").tls_extra_ca_pem(Vec::new());
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
    /// let builder = papo::Client::builder("ircs://irc.example.net", "papo-client")
    ///     .tls_client_cert(fs::read(cert)?, fs::read(key)?);
    /// # Ok(builder)
    /// # }
    /// ```
    #[cfg(not(browser))]
    pub fn tls_client_cert(
        mut self,
        certificate: impl Into<Vec<u8>>,
        key: impl Into<Vec<u8>>,
    ) -> Self {
        self.tls.client_certificate = Some(ClientCertificate::new(certificate, key));
        self
    }

    /// Overrides the DNS name authenticated by TLS.
    ///
    /// This permits connecting to an IP endpoint while verifying a certificate for a DNS name.
    /// See the private-CA example on [`Self::tls_ca_pem`].
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
    /// [`Builder::connect`] can return before these channels are joined or synchronized. Wait for
    /// [`crate::Event::ChannelSynced`] for each channel, or check that [`crate::State::channel`]
    /// returns a channel whose [`crate::Channel::is_synced`] is `true`.
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), papo::Error> {
    /// use papo::{Client, Event};
    ///
    /// let (_client, mut events) = Client::builder("ircs://irc.example.net", "papo-client")
    ///     .autojoin(["#papo"])
    ///     .connect()
    ///     .await?;
    /// while let Some(event) = events.next().await {
    ///     if matches!(event, Event::ChannelSynced(ref synced) if synced.channel == "#papo") {
    ///         break;
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn autojoin<I, S>(mut self, channels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for channel in channels {
            self.push_autojoin(Autojoin::new(channel, None));
        }
        self
    }

    /// Adds a keyed channel to join after connecting.
    ///
    /// This replaces an exact configured name. Once the server's casemapping is known, names
    /// that compare equal under that mapping are also deduplicated before joining.
    ///
    /// ```
    /// # _ = papo::Client::builder("ircs://irc.example.net", "papo-client").autojoin_with_key("#staff", papo::Secret::new("secret"));
    /// ```
    pub fn autojoin_with_key(
        mut self,
        channel: impl Into<String>,
        key: impl Into<papo_core::Secret>,
    ) -> Self {
        self.push_autojoin(Autojoin::new(channel, Some(key.into())));
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

    fn push_autojoin(&mut self, entry: Autojoin) {
        self.autojoin
            .retain(|configured| configured.channel != entry.channel);
        self.autojoin.push(entry);
    }

    /// Sets whether network reconnects fetch history missed while disconnected.
    ///
    /// Enabling this requires the server to acknowledge `draft/chathistory`; otherwise the
    /// connection fails with [`ConnectErrorKind::CapabilityUnavailable`](crate::ConnectErrorKind::CapabilityUnavailable).
    ///
    /// This emits one history event per channel joined during this process. It does not recover
    /// direct messages, channels from an earlier process, or in-memory history lost when the IRC
    /// server itself restarts. Full clients should use [`crate::Client::recent_conversations`] and
    /// [`crate::Client::history`] with [`crate::History::after`] and a persisted cursor.
    pub fn history_on_reconnect(mut self, enabled: bool) -> Self {
        self.history_on_reconnect = enabled;
        self.config
            .__set_capability_required(cap::CHATHISTORY.clone(), enabled);
        self
    }

    /// Builds handles and a lazy driver without opening a connection.
    ///
    /// The returned [`crate::Driver`] connects when it is first polled. Spawn or otherwise poll
    /// the driver before awaiting client actions, which wait for the driver to process them.
    pub fn build(self) -> (crate::Client, crate::Events, crate::Driver) {
        self.build_with_connect(None, None)
    }

    /// Connects in a spawned driver and waits until the connection is ready.
    /// IRC calls this login handshake connection registration.
    ///
    /// This method returns after registration, before configured autojoins are necessarily joined
    /// or synchronized. To wait for channel readiness, consume [`crate::Event::ChannelSynced`] or
    /// check [`crate::State::channel`] and [`crate::Channel::is_synced`], as shown by
    /// [`Builder::autojoin`].
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
    /// The connection remains alive while either returned handle exists. Dropping both sends a
    /// bare `QUIT`; use [`crate::Client::quit`] for an explicit shutdown reason and outcome.
    ///
    /// # Panics
    ///
    /// On native targets, panics when called outside a Tokio runtime because the driver is
    /// spawned before this future waits for registration.
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
        security: StreamSecurity,
        registration: Option<crate::driver::ConnectSender>,
    ) -> (crate::Client, crate::Events, crate::Driver) {
        let network_builder =
            std::sync::Arc::new(std::sync::RwLock::new(self.network_settings(false)));
        self.config.origin = allocate_origin();
        if let Ok(address) = &self.address {
            if security == StreamSecurity::Plaintext
                && (self.credentials_configured || self.config.password.is_some())
                && !self.allow_insecure_auth
            {
                self.invalid_input
                    .get_or_insert(InvalidInput::InsecureAuthentication);
            }
            apply_address(&mut self.config, address);
            self.config.connection_security = match security {
                StreamSecurity::Plaintext => ConnectionSecurity::Plaintext,
                StreamSecurity::Encrypted => ConnectionSecurity::Secure,
            };
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
        let (mut client, events, driver) = crate::Driver::with_stream_and_connect(
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
        );
        client.network_builder = Some(network_builder);
        (client, events, driver)
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
        if let Some((stream, security)) = self.stream.take() {
            return self.build_stream_with_connect(stream, security, registration);
        }
        let network_builder =
            std::sync::Arc::new(std::sync::RwLock::new(self.network_settings(true)));
        self.config.origin = allocate_origin();
        let validation_error = self
            .invalid_input
            .or_else(|| connect_input_error(&self.config));
        let plan = self.address.and_then(|address| {
            if let Some(error) = validation_error {
                return Err(error);
            }
            if !address.is_secure() && self.credentials_configured && !self.allow_insecure_auth {
                return Err(InvalidInput::InsecureAuthentication);
            }
            #[cfg(tls)]
            if address.is_secure() || self.tls.has_custom_settings() {
                crate::transport::prepare_tls_config(&address.host, &mut self.tls)
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
        let (mut client, events, driver) = crate::Driver::with_plan(
            plan,
            session,
            self.event_capacity.max(1),
            registration,
            defaults,
            self.reconnect,
            self.connect_timeout,
            self.sts_store,
            connector,
            std::sync::Arc::clone(&network_builder),
        );
        client.network_builder = Some(network_builder);
        (client, events, driver)
    }
}

#[cfg(not(browser))]
impl Builder {
    /// Uses an already-connected asynchronous byte stream.
    ///
    /// The stream is treated as plaintext. Custom streams are driven once and do not reconnect or
    /// apply STS policies. Use [`Self::stream_with_security`] when the caller established an
    /// encrypted stream.
    pub fn stream(
        self,
        stream: impl futures_io::AsyncRead + futures_io::AsyncWrite + Send + Unpin + 'static,
    ) -> Self {
        self.stream_with_security(stream, StreamSecurity::Plaintext)
    }

    /// Uses an already-connected asynchronous byte stream with explicit transport security.
    ///
    /// [`StreamSecurity::Encrypted`] is a declaration by the caller that encryption has already
    /// been established. Papo does not inspect or add encryption to a custom stream.
    pub fn stream_with_security(
        mut self,
        stream: impl futures_io::AsyncRead + futures_io::AsyncWrite + Send + Unpin + 'static,
        security: StreamSecurity,
    ) -> Self {
        self.stream = Some((Box::new(stream), security));
        self
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
    config
        .validate_registration()
        .err()
        .map(InvalidInput::WireMessage)
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
        CtcpReplies, DEFAULT_WANTED_CAPS, RateLimit, Sasl, Secret, cap, sts::ConnectionSecurity,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::{Builder, StreamSecurity, apply_address, connect_input_error};
    use crate::{
        ConnectErrorKind, Error, InvalidInput, NewAccount,
        address::Protocol,
        driver::test_support::{
            abort_task, client_line, next_event, scripted, server_line, welcome_burst, within,
        },
        transport::TlsRoots,
    };

    fn webirc(gateway: &str, password: &str) -> crate::WebIrc {
        crate::WebIrc::new(
            gateway,
            crate::Secret::new(password),
            "192.0.2.42".parse().unwrap(),
        )
    }

    async fn assert_cap_request(builder: Builder, advertised: &str, expected: &str) {
        let (mut server, _client, _events, driver) = scripted(builder.config);
        let driver = tokio::spawn(driver);
        client_line(&mut server, b"CAP LS 302\r\n").await;
        client_line(&mut server, b"NICK papo\r\n").await;
        client_line(&mut server, b"USER papo 0 * papo\r\n").await;
        server_line(
            &mut server,
            format!(":ergo.test CAP * LS :{advertised}\r\n").as_bytes(),
        )
        .await;
        client_line(&mut server, format!("CAP REQ :{expected}\r\n").as_bytes()).await;
        abort_task(driver).await;
    }

    #[test]
    fn clients_distinguish_same_named_channels() {
        let (first, _, _) = Builder::new("ircs://irc.example.net")
            .casemapping(crate::Casemapping::default())
            .build();
        let (second, _, _) = Builder::new("ircs://irc.example.net")
            .casemapping(crate::Casemapping::default())
            .build();

        assert_ne!(first.origin(), second.origin());
        let first_channel = first
            .state()
            .name_rules()
            .expect("server advertised CASEMAPPING")
            .channel("#papo");
        let second_channel = second
            .state()
            .name_rules()
            .expect("server advertised CASEMAPPING")
            .channel("#papo");
        assert_eq!(first_channel, "#papo");
        assert_eq!(second_channel, "#papo");
        assert_ne!(first_channel, second_channel);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sends_reject_values_from_another_client() {
        let (first, _, _) = Builder::new("ircs://irc.example.net")
            .casemapping(crate::Casemapping::default())
            .build();
        let (second, _, _) = Builder::new("ircs://irc.example.net")
            .casemapping(crate::Casemapping::default())
            .build();
        let reference = (
            first
                .state()
                .name_rules()
                .expect("server advertised CASEMAPPING")
                .channel("#papo"),
            "RfcBTzyxY120004",
        );

        assert!(matches!(
            second.reply(reference, "release ready").await,
            Err(crate::Error::InvalidInput(
                crate::InvalidInput::ForeignReference
            ))
        ));
        let channel = first
            .state()
            .name_rules()
            .expect("server advertised CASEMAPPING")
            .channel("#papo");
        assert!(matches!(
            second.privmsg(channel, "release ready").await,
            Err(crate::Error::InvalidInput(
                crate::InvalidInput::ForeignReference
            ))
        ));

        let actor = crate::__testing::user_source(
            first
                .state()
                .name_rules()
                .expect("server advertised CASEMAPPING")
                .nick("emily"),
            Some("~emily".into()),
            Some("cloak.example".into()),
        );
        let mut private = crate::__testing::message(
            Some(actor.clone()),
            first.nick().to_string(),
            "release ready",
            true,
        );
        private.conversation = crate::Conversation::Private(
            first
                .state()
                .name_rules()
                .expect("server advertised CASEMAPPING")
                .nick("emily"),
        );
        private.receipt = crate::message_ref::Receipt::new(
            crate::Source::User(private.actor.clone()),
            private.authorship,
            &private.conversation,
            None,
            "emily".into(),
        );
        assert_eq!(private.origin(), first.origin());
        assert!(matches!(
            second.reply(&private, "ship it").await,
            Err(crate::Error::InvalidInput(
                crate::InvalidInput::ForeignReference
            ))
        ));

        let conversation = crate::Conversation::Private(
            first
                .state()
                .name_rules()
                .expect("server advertised CASEMAPPING")
                .nick("emily"),
        );
        let receipt = crate::message_ref::Receipt::new(
            crate::Source::User(actor.clone()),
            crate::Authorship::Other,
            &conversation,
            None,
            "emily".into(),
        );
        let tag = crate::event::TagMsg {
            msgid: None,
            server_time: None,
            received_at: std::time::SystemTime::UNIX_EPOCH,
            actor: actor.clone(),
            authorship: crate::Authorship::Other,
            conversation,
            status: None,
            receipt,
            tags: crate::Tags::default(),
        };
        assert!(matches!(
            second.reply(&tag, "ship it").await,
            Err(crate::Error::InvalidInput(
                crate::InvalidInput::ForeignReference
            ))
        ));
    }

    #[cfg(tls)]
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn connect_timeout_covers_a_silent_tls_peer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
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
        inner: crate::driver::test_support::TokioIo<tokio::io::DuplexStream>,
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
        let builder = Builder::from_endpoint(
            "ircs://irc.example.net",
            crate::Casemapping::default().nick("ping-bot"),
        )
        .casemapping(crate::Casemapping::Ascii)
        .alt_nicks([
            crate::Casemapping::default().nick("ping-bot_"),
            crate::Casemapping::default().nick("ping-bot__"),
        ])
        .regain_nick(true)
        .username("ping")
        .realname("papo ping bot")
        .password(crate::Secret::new("network password"))
        .sasl(
            Sasl::plain(
                "ping-bot",
                crate::Secret::new("correct horse battery staple"),
            )
            .device("phone"),
        );
        let config = &builder.config;
        assert_eq!(config.casemapping, crate::Casemapping::Ascii);
        assert_eq!(config.nick, "ping-bot");
        assert_eq!(config.alt_nicks, ["ping-bot_", "ping-bot__"]);
        assert!(config.regain_nick);
        assert_eq!(config.username, "ping");
        assert_eq!(config.realname, "papo ping bot");
        assert_eq!(config.password, Some(Secret::new("network password")));
        assert!(config.sasl.is_some());
        assert_eq!(
            config.wanted_caps[..DEFAULT_WANTED_CAPS.len()],
            DEFAULT_WANTED_CAPS[..]
        );
        assert_eq!(config.wanted_caps.len(), DEFAULT_WANTED_CAPS.len());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn configured_nickname_regain_reaches_the_wire() {
        let mut builder = Builder::from_endpoint("ircs://irc.example.net", "papo-driver")
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
            next_event(&mut events).await,
            Some(crate::Event::Connected(_))
        ));
        client_line(&mut server, b"MONITOR + papo-driver\r\n").await;
        server_line(&mut server, b":ergo.test 731 papo-driver_ :papo-driver\r\n").await;
        client_line(&mut server, b"NICK papo-driver\r\n").await;
        assert!(matches!(
            next_event(&mut events).await,
            Some(crate::Event::MonitorOffline(_))
        ));
        abort_task(driver).await;
    }

    #[test]
    fn seeds_offline_state_name_rules() {
        let (client, _events, _driver) = Builder::from_endpoint(
            "ircs://irc.example.net",
            crate::Casemapping::Ascii.nick("papo[bot]"),
        )
        .casemapping(crate::Casemapping::Ascii)
        .build();

        let state = client.state();
        assert!(!state.is_connected());
        assert_eq!(state.casemapping(), crate::Casemapping::Ascii);
        assert_eq!(state.nick().casemapping(), crate::Casemapping::Ascii);
        assert_eq!(
            state
                .name_rules()
                .expect("builder seeded CASEMAPPING")
                .channel("#Papo[dev]"),
            "#papo[dev]"
        );
        assert_ne!(state.nick(), "papo{bot}");
    }

    #[test]
    fn account_registration_reenables_its_capability() {
        let added = Builder::new("ircs://irc.example.net")
            .disable(crate::Feature::AccountCreation)
            .create_account(NewAccount::new(crate::Secret::new("realistic secret")))
            .config;
        assert_eq!(
            added
                .wanted_caps
                .iter()
                .filter(|name| name.as_str() == "draft/account-registration")
                .count(),
            1
        );

        assert!(
            !added.disabled_caps.iter().any(|name| {
                cap::canonical(name) == cap::canonical("draft/account-registration")
            })
        );
    }

    #[test]
    fn unknown_capabilities_extend_defaults_and_features_toggle_known_caps() {
        let builder = Builder::new("ircs://irc.example.net")
            .request_caps(["example.com/custom-cap"])
            .disable(crate::Feature::AwayTracking);
        assert!(
            builder
                .config
                .wanted_caps
                .iter()
                .any(|name| name == "example.com/custom-cap")
        );
        assert!(
            builder
                .config
                .disabled_caps
                .iter()
                .any(|name| name == "away-notify")
        );

        let enabled = builder.enable(crate::Feature::AwayTracking);
        assert!(
            !enabled
                .config
                .disabled_caps
                .iter()
                .any(|name| name == "away-notify")
        );
    }

    #[test]
    fn known_capabilities_record_the_typed_configuration_error() {
        let known = Builder::new("ircs://irc.example.net").request_caps(["chathistory"]);
        assert_eq!(
            known.invalid_input,
            Some(InvalidInput::KnownCapability {
                capability: "chathistory".to_owned(),
                feature: "History",
            })
        );

        let managed = Builder::new("ircs://irc.example.net").request_caps(["sts"]);
        assert_eq!(
            managed.invalid_input,
            Some(InvalidInput::ManagedCapability("sts".to_owned()))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unknown_capability_is_requested_when_ergo_advertises_it() {
        assert_cap_request(
            Builder::new("ircs://irc.example.net").request_caps(["vendor.example/network-state"]),
            "account-tag vendor.example/network-state",
            "account-tag vendor.example/network-state",
        )
        .await;
    }

    #[cfg(not(feature = "unstable"))]
    #[test]
    fn unstable_capabilities_record_the_cargo_feature_error() {
        let builder = Builder::new("ircs://irc.example.net").request_caps(["draft/persistence"]);
        assert_eq!(
            builder.invalid_input,
            Some(InvalidInput::UnstableCapability(
                "draft/persistence".to_owned()
            ))
        );
    }

    #[cfg(feature = "unstable")]
    #[tokio::test(flavor = "current_thread")]
    #[allow(clippy::too_many_lines)]
    async fn enables_unstable_features_and_bouncer_selection_before_connecting() {
        let known = Builder::new("ircs://irc.example.net").request_caps(["draft/persistence"]);
        assert_eq!(
            known.invalid_input,
            Some(InvalidInput::KnownCapability {
                capability: "draft/persistence".to_owned(),
                feature: "Persistence",
            })
        );

        let advertised = "soju.im/bouncer-networks soju.im/bouncer-networks-notify";
        assert_cap_request(
            Builder::new("ircs://irc.example.net")
                .bouncer_network(crate::NetworkId::from("libera")),
            advertised,
            "soju.im/bouncer-networks",
        )
        .await;
        assert_cap_request(
            Builder::new("ircs://irc.example.net")
                .bouncer_network(crate::NetworkId::from("libera"))
                .enable(crate::Feature::Bouncer),
            advertised,
            "soju.im/bouncer-networks soju.im/bouncer-networks-notify",
        )
        .await;
        assert_cap_request(
            Builder::new("ircs://irc.example.net")
                .enable(crate::Feature::Bouncer)
                .bouncer_network(crate::NetworkId::from("libera")),
            advertised,
            "soju.im/bouncer-networks soju.im/bouncer-networks-notify",
        )
        .await;

        let notify = "soju.im/bouncer-networks-notify";
        let management = Builder::new("ircs://irc.example.net").enable(crate::Feature::Bouncer);
        assert!(
            management
                .config
                .wanted_caps
                .iter()
                .any(|cap| cap == notify)
        );
        assert!(management.config.disabled_caps.is_empty());
        let bound = Builder::new("ircs://irc.example.net")
            .bouncer_network(crate::NetworkId::from("libera"));
        assert!(!bound.config.wanted_caps.iter().any(|cap| cap == notify));

        let enabled = Builder::new("ircs://irc.example.net")
            .enable(crate::Feature::Bouncer)
            .enable(crate::Feature::Persistence)
            .enable(crate::Feature::Relay)
            .enable(crate::Feature::WebPush)
            .bouncer_network(crate::NetworkId::from("libera"));
        assert!(enabled.config.disabled_caps.is_empty());
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
        let opted_in = Builder::new("ircs://irc.example.net")
            .bouncer_network(crate::NetworkId::from("libera"))
            .enable(crate::Feature::Bouncer);
        assert!(opted_in.config.wanted_caps.iter().any(|cap| cap == notify));
        assert!(opted_in.config.disabled_caps.is_empty());
        let other_order = Builder::new("ircs://irc.example.net")
            .enable(crate::Feature::Bouncer)
            .bouncer_network(crate::NetworkId::from("libera"));
        assert_eq!(opted_in.config.wanted_caps, other_order.config.wanted_caps);

        let reconnect = crate::Reconnect::never();
        let (control, _events, _driver) =
            Builder::from_endpoint("ircs://soju.example:7443", "alice")
                .password(crate::Secret::new("correct horse battery staple"))
                .rate_limit(RateLimit::unlimited())
                .reconnect(reconnect)
                .connect_timeout(Duration::from_secs(17))
                .tls_server_name("bouncer.example")
                .enable(crate::Feature::Relay)
                .metadata_subscriptions([crate::MetadataKey::new("example.org/color")])
                .event_capacity(17)
                .autojoin(["#papo"])
                .history_on_reconnect(true)
                .build();

        let network = control.bouncer().network(&crate::NetworkId::from("libera"));
        assert_eq!(network.address.as_ref().unwrap().port, 7443);
        assert_eq!(network.config.nick, "alice");
        assert_eq!(
            network.config.password.as_ref().unwrap().expose(),
            "correct horse battery staple"
        );
        assert_eq!(network.config.rate_limit, RateLimit::unlimited());
        assert!(
            network
                .config
                .wanted_caps
                .iter()
                .any(|name| name == "draft/relaymsg")
        );
        assert_eq!(network.config.bouncer_network.as_deref(), Some("libera"));
        assert!(network.config.metadata_subscriptions.is_empty());
        assert_eq!(
            network.event_capacity,
            crate::driver::DEFAULT_CHANNEL_CAPACITY
        );
        assert!(network.autojoin.is_empty());
        assert!(!network.history_on_reconnect);
        assert_eq!(network.connect_timeout, Duration::from_secs(17));
        assert_eq!(network.reconnect, reconnect);
        assert_eq!(network.tls.server_name.as_deref(), Some("bouncer.example"));

        let (stream, _server) = tokio::io::duplex(64);
        let (control, _events, _driver) = Builder::new("irc://custom.example")
            .stream(crate::driver::test_support::TokioIo(stream))
            .build();
        let error = control
            .bouncer()
            .network(&crate::NetworkId::from("libera"))
            .connect()
            .await
            .err()
            .unwrap();
        assert!(matches!(
            error,
            Error::InvalidInput(InvalidInput::CustomStreamNotReusable)
        ));
    }

    #[test]
    fn required_nick_supplies_implicit_username_and_realname_defaults() {
        let implicit = Builder::from_endpoint(
            "ircs://irc.example.net",
            crate::Casemapping::default().nick("ping-bot"),
        )
        .config;
        assert_eq!(
            (implicit.username.as_str(), implicit.realname.as_str()),
            ("ping-bot", "ping-bot")
        );
        let explicit = Builder::from_endpoint(
            "ircs://irc.example.net",
            crate::Casemapping::default().nick("ping-bot"),
        )
        .username("service")
        .realname("Papo service")
        .config;
        assert_eq!(
            (explicit.username.as_str(), explicit.realname.as_str()),
            ("service", "Papo service")
        );
    }

    #[test]
    fn configures_timing_rate_and_event_capacity() {
        assert_eq!(
            Builder::new("ircs://irc.example.net")
                .config
                .request_timeout,
            Duration::from_secs(30)
        );
        let builder = Builder::new("ircs://irc.example.net")
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
    fn clamps_event_capacity_to_the_runtime_limit() {
        let builder = Builder::new("ircs://irc.example.net").event_capacity(usize::MAX);
        assert_eq!(builder.event_capacity, tokio::sync::Semaphore::MAX_PERMITS);
    }

    #[test]
    fn configures_away_and_ctcp_replies() {
        let rate = RateLimit::new(4, Duration::from_secs(3)).unwrap();
        assert_eq!(
            Builder::new("ircs://irc.example.net").config.ctcp,
            CtcpReplies::default()
        );
        let replies = CtcpReplies::default()
            .without_version()
            .ping(false)
            .rate(rate);
        let builder = Builder::new("ircs://irc.example.net")
            .away_on_connect("Reviewing patches")
            .ctcp_replies(replies.clone());
        assert_eq!(
            builder.config.away_on_connect,
            Some(papo_core::AwayOnConnect::Message(
                "Reviewing patches".into()
            ))
        );
        assert_eq!(builder.config.ctcp, replies);
    }

    #[test]
    fn debug_is_opaque_with_every_builder_secret() {
        let builder = Builder::new("ircs://irc.example.net")
            .password(crate::Secret::new("unique-network-password-9134"))
            .webirc(webirc("papo-gateway", "unique-webirc-password-3186"))
            .sasl(Sasl::plain(
                "alice",
                crate::Secret::new("unique-sasl-password-8527"),
            ))
            .create_account(NewAccount::new(crate::Secret::new(
                "unique-account-password-7416",
            )))
            .tls_client_cert(Vec::new(), b"unique-tls-key-6305".to_vec())
            .autojoin_with_key("#staff", crate::Secret::new("unique-channel-key-5294"));
        let output = format!("{builder:?}");
        assert_eq!(output, "Builder { .. }");
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
        let cases = vec![
            (
                Builder::new("ircs://irc.example.net").event_capacity(0),
                InvalidInput::ZeroEventCapacity,
            ),
            (
                Builder::new("ircs://irc.example.net").away_on_connect(""),
                InvalidInput::EmptyText,
            ),
            (
                Builder::new("ircs://irc.example.net").request_caps(["chathistory"]),
                InvalidInput::KnownCapability {
                    capability: "chathistory".to_owned(),
                    feature: "History",
                },
            ),
            (
                Builder::new("ircs://irc.example.net").request_caps(["sts"]),
                InvalidInput::ManagedCapability("sts".to_owned()),
            ),
        ];
        #[cfg(not(feature = "unstable"))]
        let cases = cases
            .into_iter()
            .chain([(
                Builder::new("ircs://irc.example.net").request_caps(["draft/persistence"]),
                InvalidInput::UnstableCapability("draft/persistence".to_owned()),
            )])
            .collect::<Vec<_>>();
        for (builder, expected) in cases {
            let (_client, _events, driver) = builder.build();
            assert!(matches!(
                within(driver).await,
                Err(Error::InvalidInput(actual)) if actual == expected
            ));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn plaintext_endpoints_reject_credentials_by_default() {
        let webirc = || webirc("papo-gateway", "gateway-password");
        let cases = [
            Builder::new("ws://irc.example.net/webirc").webirc(webirc()),
            Builder::new("irc://irc.example.net")
                .sasl(Sasl::plain("alice", crate::Secret::new("sasl-password"))),
            Builder::new("irc://irc.example.net").create_account(
                NewAccount::new(crate::Secret::new("account-password")).account("alice"),
            ),
        ];
        for builder in cases {
            let (_client, _events, driver) = builder.build();
            assert!(matches!(within(driver).await, Err(Error::InvalidInput(_))));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_invalid_registration_messages_before_connecting() {
        let cases = [
            Builder::from_endpoint(
                "ircs://irc.example.net",
                crate::Casemapping::default().nick("papo\rOPER root password"),
            ),
            Builder::new("ircs://irc.example.net")
                .alt_nicks([crate::Casemapping::default().nick("papo-away\nQUIT")]),
            Builder::new("ircs://irc.example.net").username("papo\0admin"),
            Builder::new("ircs://irc.example.net").realname("Papo\rClient"),
            Builder::new("ircs://irc.example.net")
                .password(crate::Secret::new("network\npassword")),
            Builder::new("ircs://irc.example.net")
                .webirc(webirc("papo\rgateway", "gateway-password")),
            Builder::new("ircs://irc.example.net").away_on_connect("gone\nAWAY"),
            Builder::new("ircs://irc.example.net")
                .username("u".repeat(250))
                .realname("r".repeat(251)),
        ];
        for builder in cases {
            let (_client, _events, driver) = builder.build();
            assert!(matches!(
                within(driver).await,
                Err(Error::InvalidInput(InvalidInput::WireMessage(_)))
            ));
        }
    }

    #[test]
    fn accepts_webirc_without_options() {
        let config = Builder::new("ircs://irc.example.net")
            .webirc(webirc("papo-gateway", "gateway-password"))
            .config;
        assert_eq!(connect_input_error(&config), None);
    }

    #[test]
    fn address_security_reaches_the_session_configuration() {
        for (input, expected) in [
            ("ircs://irc.example.net", ConnectionSecurity::Secure),
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
        let builder = Builder::new("ircs://irc.example.net")
            .tls_ca_pem(b"replaced CA".to_vec())
            .tls_extra_ca_pem(b"test CA".to_vec())
            .tls_client_cert(b"certificate".to_vec(), b"private key".to_vec())
            .tls_server_name("chat.example.net");
        assert!(matches!(builder.tls.roots, TlsRoots::PlatformAndCa(pem) if pem == b"test CA"));
        assert!(builder.tls.client_certificate.is_some());
        assert_eq!(builder.tls.server_name.as_deref(), Some("chat.example.net"));

        let builder = Builder::new("ircs://irc.example.net")
            .tls_extra_ca_pem(b"replaced extra CA".to_vec())
            .tls_ca_pem(b"only CA".to_vec());
        assert!(matches!(builder.tls.roots, TlsRoots::Ca(pem) if pem == b"only CA"));
    }

    #[test]
    fn configures_post_registration_defaults() {
        assert_eq!(
            Builder::new("ircs://irc.example.net").config.who_on_join,
            crate::WhoOnJoin::Always
        );
        let staff = crate::Casemapping::default().channel("#staff");
        assert!(!Builder::new("ircs://irc.example.net").bot(false).bot);
        let builder = Builder::new("ircs://irc.example.net")
            .bot(true)
            .autojoin(["#papo", "#papo-bots"])
            .autojoin_with_key(staff, crate::Secret::new("private channel key"))
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
        let builder = Builder::new("ircs://irc.example.net")
            .autojoin(["#papo", "#papo", "#PAPO"])
            .autojoin_with_key("#papo", crate::Secret::new("private channel key"));
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
        assert!(next_event(&mut events).await.is_none());
    }

    #[cfg(tls)]
    #[tokio::test(flavor = "current_thread")]
    async fn build_validates_every_tls_setting_for_every_scheme() {
        const CERTIFICATE: &[u8] = include_bytes!("../tests/data/client.pem");
        const PRIVATE_KEY: &[u8] = include_bytes!("../tests/data/client.key");

        fn bad_ca(builder: Builder) -> Builder {
            builder.tls_ca_pem(
                b"-----BEGIN CERTIFICATE-----\nnot-base64!\n-----END CERTIFICATE-----\n",
            )
        }
        fn bad_certificate(builder: Builder) -> Builder {
            builder.tls_client_cert(b"not a certificate", PRIVATE_KEY)
        }
        fn bad_private_key(builder: Builder) -> Builder {
            builder.tls_client_cert(CERTIFICATE, b"not a private key")
        }
        fn bad_server_name(builder: Builder) -> Builder {
            builder.tls_server_name("not a server name")
        }

        type TlsCase = (fn(Builder) -> Builder, &'static str);
        let default = Builder::new("irc://irc.example.net");
        assert!(matches!(default.tls.roots, TlsRoots::Platform));
        assert!(!default.tls.has_custom_settings());

        let cases: [TlsCase; 4] = [
            (bad_ca, "invalid certificate PEM"),
            (bad_certificate, "certificate PEM contains no certificates"),
            (bad_private_key, "invalid client private key PEM"),
            (bad_server_name, "invalid TLS server name"),
        ];
        for scheme in ["irc", "ircs", "ws", "wss"] {
            for (configure, expected) in cases {
                let endpoint = format!("{scheme}://irc.example.net");
                let (_client, _events, driver) =
                    configure(Builder::from_endpoint(endpoint.as_str(), "papo-client"))
                        .reconnect(crate::Reconnect::never())
                        .build();
                assert!(
                    matches!(
                        within(driver).await,
                        Err(Error::InvalidInput(InvalidInput::TlsConfig(message)))
                            if message.contains(expected)
                    ),
                    "accepted {scheme} configuration with {expected}"
                );
            }
        }
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
            let (_client, _events, driver) = crate::Client::builder(input, "papo-client").build();
            assert!(
                matches!(within(driver).await, Err(Error::InvalidInput(_))),
                "accepted {input:?}"
            );
        }
        let (_client, _events, driver) =
            crate::Client::builder("irc.example.net", "papo-client").build();
        assert_eq!(
            within(driver).await.unwrap_err().to_string(),
            "server URL scheme is missing; use ircs:// for TLS"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn public_builder_maps_each_server_url_error_exactly() {
        let cases = [
            (
                "",
                InvalidInput::Url(crate::InvalidUrl::Empty),
                "server URL is empty",
            ),
            (
                "irc://",
                InvalidInput::Url(crate::InvalidUrl::EmptyHost),
                "server URL host is empty",
            ),
            (
                "irc://alice:secret@irc.oftc.net",
                InvalidInput::Url(crate::InvalidUrl::Userinfo),
                "server URL cannot contain user information",
            ),
            (
                "irc://irc.oftc.net?network=oftc",
                InvalidInput::Url(crate::InvalidUrl::Query),
                "server URL cannot contain a query",
            ),
            (
                "irc://irc.oftc.net#support",
                InvalidInput::Url(crate::InvalidUrl::Fragment),
                "server URL cannot contain a fragment",
            ),
            (
                "irc://irc.oftc.net /support",
                InvalidInput::Url(crate::InvalidUrl::Whitespace),
                "server URL cannot contain whitespace",
            ),
            (
                "irc://irc.oftc.net\u{7f}",
                InvalidInput::Url(crate::InvalidUrl::ControlCharacter),
                "server URL cannot contain control characters",
            ),
            (
                "irc://irc%2Eoftc.net",
                InvalidInput::Url(crate::InvalidUrl::PercentEncoding),
                "server URL cannot contain percent-encoding",
            ),
            (
                "irc://irc.réseau.example",
                InvalidInput::Url(crate::InvalidUrl::NonAsciiHost("irc.réseau.example".into())),
                "non-ASCII server host \"irc.réseau.example\"; use its punycode xn-- form",
            ),
            (
                "irc://irc.oftc.net.",
                InvalidInput::Url(crate::InvalidUrl::TrailingDot("irc.oftc.net.".into())),
                "server host \"irc.oftc.net.\" has a trailing dot",
            ),
            (
                "irc://[fe80::1%25eth0]",
                InvalidInput::Url(crate::InvalidUrl::ZoneIdentifier("fe80::1%25eth0".into())),
                "IPv6 zone identifiers are not supported in \"fe80::1%25eth0\"",
            ),
            (
                "irc://2001:db8::6697",
                InvalidInput::Url(crate::InvalidUrl::UnbracketedIpv6("2001:db8::6697".into())),
                "IPv6 literal \"2001:db8::6697\" must be enclosed in brackets",
            ),
            (
                "irc://[2001:db8::support]",
                InvalidInput::Url(crate::InvalidUrl::InvalidHost("2001:db8::support".into())),
                "invalid server host \"2001:db8::support\"",
            ),
            (
                "ircs://irc.oftc.net:70000",
                InvalidInput::Url(crate::InvalidUrl::InvalidPort("70000".into())),
                "invalid service port \"70000\"",
            ),
            (
                "ircs://irc.oftc.net/support",
                InvalidInput::Url(crate::InvalidUrl::UnexpectedPath("/support".into())),
                "IRC server URL cannot contain path \"/support\"",
            ),
        ];

        for (input, expected, message) in cases {
            let (_client, _events, driver) = crate::Client::builder(input, "papo-client").build();
            let error = within(driver).await.unwrap_err();
            let Error::InvalidInput(actual) = error else {
                panic!("wrong error kind for {input:?}: {error:?}");
            };
            assert_eq!(actual, expected, "wrong error for {input:?}");
            assert_eq!(actual.to_string(), message, "wrong message for {input:?}");
        }
    }

    #[test]
    fn public_builder_accepts_owned_and_borrowed_server_addresses() {
        let borrowed_address = "ws://irc.example.net:8080/socket".to_owned();
        let borrowed_nick = "papo-client".to_owned();
        for (builder, protocol, port) in [
            (
                crate::Client::builder("ircs://irc.example.net", &borrowed_nick),
                Protocol::Ircs,
                6697,
            ),
            (
                crate::Client::builder(
                    String::from("irc://irc.example.net:7000"),
                    borrowed_nick.clone(),
                ),
                Protocol::Irc,
                7000,
            ),
            (
                crate::Client::builder(&borrowed_address, "papo-client"),
                Protocol::Ws,
                8080,
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
        let error = within(
            Builder::new("ircs://irc.example.net")
                .event_capacity(0)
                .connect(),
        )
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
                .stream(crate::driver::test_support::TokioIo(stream))
                .connect(),
        )
        .await
        .expect_err("an early peer close must fail connect");
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
    async fn custom_stream_is_plaintext_despite_a_secure_url() {
        let (stream, mut server) = tokio::io::duplex(4096);
        let (_client, _events, driver) = Builder::new("ircs://custom.example")
            .stream(crate::driver::test_support::TokioIo(stream))
            .sasl(Sasl::plain(
                "alice",
                crate::Secret::new("stream-password-8732"),
            ))
            .build();

        assert!(matches!(
            within(driver).await,
            Err(Error::InvalidInput(InvalidInput::InsecureAuthentication))
        ));
        let mut output = Vec::new();
        within(server.read_to_end(&mut output)).await.unwrap();
        assert!(output.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn encrypted_custom_stream_accepts_credentials() {
        let (stream, mut server) = tokio::io::duplex(4096);
        let (_client, _events, driver) = Builder::new("irc://custom.example")
            .stream_with_security(
                crate::driver::test_support::TokioIo(stream),
                StreamSecurity::Encrypted,
            )
            .sasl(Sasl::plain(
                "alice",
                crate::Secret::new("stream-password-8732"),
            ))
            .build();
        let driver = tokio::spawn(driver);

        client_line(&mut server, b"CAP LS 302\r\n").await;
        client_line(&mut server, b"NICK papo\r\n").await;
        client_line(&mut server, b"USER papo 0 * papo\r\n").await;
        server_line(&mut server, b":custom.example CAP * LS :sasl=PLAIN\r\n").await;
        client_line(&mut server, b"CAP REQ :sasl\r\n").await;
        server_line(&mut server, b":custom.example CAP * ACK :sasl\r\n").await;
        client_line(&mut server, b"AUTHENTICATE PLAIN\r\n").await;
        abort_task(driver).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn custom_stream_registers_once_without_reconnecting() {
        let (stream, mut server) = tokio::io::duplex(4096);
        let lifecycle = Arc::new(AtomicUsize::new(0));
        let connecting = tokio::spawn(
            Builder::from_endpoint(
                "irc://custom.example",
                crate::Casemapping::default().nick("papo-custom"),
            )
            .stream(ObservedStream {
                inner: crate::driver::test_support::TokioIo(stream),
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
            next_event(&mut events).await,
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
            next_event(&mut events).await,
            Some(crate::Event::Disconnected(disconnected)) if !disconnected.is_reconnecting()
        ));
        assert!(!client.state().is_connected());
        assert_eq!(lifecycle.load(Ordering::Relaxed), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn custom_stream_build_defers_address_errors() {
        let (stream, _server) = tokio::io::duplex(64);
        let (_client, mut events, driver) = Builder::new("irc://")
            .stream(crate::driver::test_support::TokioIo(stream))
            .build();
        assert!(matches!(within(driver).await, Err(Error::InvalidInput(_))));
        assert!(next_event(&mut events).await.is_none());
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
