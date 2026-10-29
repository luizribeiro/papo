use std::future::IntoFuture;

use crate::{BouncerNetwork, Client, Error, NetworkId, driver::unstable::UnstableRequest};

#[derive(Clone)]
pub(crate) struct NetworkConfig {
    host: Option<String>,
    name: Option<String>,
    port: Option<u16>,
    tls: Option<bool>,
    nick: Option<String>,
    username: Option<String>,
    realname: Option<String>,
    password: Option<crate::Secret>,
}

impl NetworkConfig {
    fn from_endpoint(endpoint: &crate::Endpoint) -> Result<Self, crate::InvalidInput> {
        let tls = match endpoint.kind() {
            crate::EndpointKind::Plaintext => false,
            crate::EndpointKind::Tls => true,
            crate::EndpointKind::WebSocket => {
                return Err(crate::InvalidUrl::UnsupportedScheme("ws".into()).into());
            }
            crate::EndpointKind::SecureWebSocket => {
                return Err(crate::InvalidUrl::UnsupportedScheme("wss".into()).into());
            }
        };
        Ok(Self {
            host: Some(endpoint.host().to_owned()),
            name: None,
            port: Some(endpoint.port()),
            tls: Some(tls),
            nick: None,
            username: None,
            realname: None,
            password: None,
        })
    }

    const fn change() -> Self {
        Self {
            host: None,
            name: None,
            port: None,
            tls: None,
            nick: None,
            username: None,
            realname: None,
            password: None,
        }
    }

    pub(crate) fn attributes(&self) -> Result<papo_proto::Param, papo_core::RequestError> {
        papo_core::__bouncer_attributes(
            self.host.as_deref(),
            self.name.as_deref(),
            self.port,
            self.tls,
            self.nick.as_deref(),
            self.username.as_deref(),
            self.realname.as_deref(),
            self.password.as_ref(),
        )
    }
}

macro_rules! network_setters {
    () => {
        /// Sets the human-readable network name.
        pub fn name(mut self, name: impl Into<String>) -> Self {
            if let Ok(config) = &mut self.config {
                config.name = Some(name.into());
            }
            self
        }

        /// Sets the connection nickname as text under the upstream network's name rules.
        pub fn nick(mut self, nick: impl Into<String>) -> Self {
            if let Ok(config) = &mut self.config {
                config.nick = Some(nick.into());
            }
            self
        }

        /// Sets the connection username.
        pub fn username(mut self, username: impl Into<String>) -> Self {
            if let Ok(config) = &mut self.config {
                config.username = Some(username.into());
            }
            self
        }

        /// Sets the connection real name.
        pub fn realname(mut self, realname: impl Into<String>) -> Self {
            if let Ok(config) = &mut self.config {
                config.realname = Some(realname.into());
            }
            self
        }

        /// Sets the upstream server password.
        pub fn password(mut self, password: crate::Secret) -> Self {
            if let Ok(config) = &mut self.config {
                config.password = Some(password);
            }
            self
        }
    };
}

/// Builds a request to add an upstream bouncer network.
#[must_use = "requests do nothing unless awaited"]
pub struct AddNetwork {
    client: Client,
    config: Result<NetworkConfig, crate::InvalidInput>,
}

impl std::fmt::Debug for AddNetwork {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("AddNetwork", formatter)
    }
}

impl AddNetwork {
    network_setters!();
}

impl IntoFuture for AddNetwork {
    type Output = Result<NetworkId, Error>;
    type IntoFuture = crate::RequestFuture<NetworkId>;

    fn into_future(self) -> Self::IntoFuture {
        crate::RequestFuture::new(async move {
            let config = self.config?;
            let response = self
                .client
                .core_request(UnstableRequest::BouncerAdd(config))
                .await?;
            response
                .lines
                .iter()
                .find_map(|message| {
                    let message = message.as_ref();
                    (message.command_is("BOUNCER")
                        && message
                            .param(0)
                            .is_some_and(|value| value.eq_ignore_ascii_case("ADDNETWORK")))
                    .then(|| message.param(1).map(NetworkId::from))
                    .flatten()
                })
                .ok_or_else(|| Error::unexpected_reply("BOUNCER"))
        })
    }
}

/// Builds a request that changes only the selected attributes of a bouncer network.
#[must_use = "requests do nothing unless awaited"]
pub struct ChangeNetwork {
    client: Client,
    id: NetworkId,
    config: Result<NetworkConfig, crate::InvalidInput>,
}

impl std::fmt::Debug for ChangeNetwork {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("ChangeNetwork", formatter)
    }
}

impl ChangeNetwork {
    network_setters!();
}

impl IntoFuture for ChangeNetwork {
    type Output = Result<(), Error>;
    type IntoFuture = crate::RequestFuture<()>;

    fn into_future(self) -> Self::IntoFuture {
        crate::RequestFuture::new(async move {
            let config = self.config?;
            let response = self
                .client
                .core_request(UnstableRequest::BouncerChange(self.id, config))
                .await?;
            crate::requests::empty(&response)
        })
    }
}

/// Handle for upstream bouncer network management.
///
/// A control connection can discover networks and create separate device connections to one of
/// them. Configure [`crate::Sasl::device`] on each network-bound builder returned by
/// [`Self::network`]. Network names need not be unique, so applications that select by name must
/// define how to resolve multiple matches.
///
/// ```no_run
/// use papo::{Client, Feature, Sasl, Secret};
///
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let password = "correct horse battery staple";
/// let (control, _control_events) = Client::builder("ircs://bouncer.example.net", "alice")
///     .sasl(Sasl::plain("alice", Secret::new(password)))
///     .enable(Feature::Bouncer)
///     .connect()
///     .await?;
///
/// let networks = control.bouncer().networks().await?;
/// let network = networks
///     .iter()
///     .find(|network| network.id == "libera")
///     .or_else(|| {
///         networks
///             .iter()
///             .find(|network| network.name.as_deref() == Some("Libera Chat"))
///     })
///     .expect("the requested network is configured");
///
/// let (_phone, _phone_events) = control
///     .bouncer()
///     .network(&network.id)
///     .sasl(Sasl::plain("alice", Secret::new(password)).device("phone"))
///     .autojoin(["#papo"])
///     .connect()
///     .await?;
/// let (_laptop, _laptop_events) = control
///     .bouncer()
///     .network(&network.id)
///     .sasl(Sasl::plain("alice", Secret::new(password)).device("laptop"))
///     .autojoin(["#papo"])
///     .connect()
///     .await?;
/// # Ok(())
/// # }
/// ```
pub struct Bouncer {
    client: Client,
}

impl std::fmt::Debug for Bouncer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("Bouncer", formatter)
    }
}

impl Client {
    /// Returns a handle for upstream bouncer network management.
    #[must_use]
    pub fn bouncer(&self) -> Bouncer {
        Bouncer {
            client: self.clone(),
        }
    }
}

impl Bouncer {
    /// Starts a client builder for one network on this bouncer.
    ///
    /// The builder copies the control connection's resolved endpoint, credentials, TLS settings,
    /// rate limit, reconnect policy, and enabled features. Per-network settings such as autojoin,
    /// metadata subscriptions, event capacity, and reconnect history keep their defaults.
    /// The bound connection verifies the selected ID and leaves bouncer network notifications on
    /// the control connection unless [`crate::Builder::enable`] explicitly enables them.
    /// A control client built from a custom stream produces a builder whose connection fails with
    /// [`crate::Error::InvalidInput`].
    pub fn network(&self, id: &NetworkId) -> crate::Builder {
        let Some(settings) = &self.client.network_builder else {
            return crate::Builder::new("ircs://irc.example.net").bouncer_network(id.clone());
        };
        let settings = settings
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        crate::Builder::from_network_settings(&settings).bouncer_network(id.clone())
    }

    /// Requests and returns the complete upstream network list.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Unsupported`] for [`crate::Feature::Bouncer`] or an error documented by
    /// [`Client::request_raw`].
    pub fn networks(&self) -> crate::RequestFuture<Vec<BouncerNetwork>> {
        let client = self.client.clone();
        crate::RequestFuture::new(async move {
            client
                .core_request(UnstableRequest::BouncerNetworks)
                .await?;
            Ok(client.state().bouncer_networks().cloned().collect())
        })
    }

    /// Adds an upstream network and returns its bouncer-assigned identifier.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for an invalid endpoint or password,
    /// [`Error::Unsupported`] for [`crate::Feature::Bouncer`], [`Error::Server`] when refused, or
    /// an error documented by [`Client::request_raw`].
    pub fn add(&self, endpoint: impl crate::IntoEndpoint) -> AddNetwork {
        let config = crate::address::unstable::endpoint(endpoint)
            .and_then(|endpoint| NetworkConfig::from_endpoint(&endpoint));
        AddNetwork {
            client: self.client.clone(),
            config,
        }
    }

    /// Starts a request that merges selected attributes into an upstream network.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for an invalid password, [`Error::Unsupported`] for
    /// [`crate::Feature::Bouncer`], [`Error::Server`] when refused, or an error documented by
    /// [`Client::request_raw`].
    pub fn change(&self, id: &NetworkId) -> ChangeNetwork {
        ChangeNetwork {
            client: self.client.clone(),
            id: id.clone(),
            config: Ok(NetworkConfig::change()),
        }
    }

    /// Removes an upstream network.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Bouncer`], [`Error::Server`] when
    /// refused, or an error documented by [`Client::request_raw`].
    pub fn remove(&self, id: &NetworkId) -> crate::RequestFuture<()> {
        let client = self.client.clone();
        let id = id.clone();
        crate::RequestFuture::new(async move {
            let response = client
                .core_request(UnstableRequest::BouncerRemove(id))
                .await?;
            crate::requests::empty(&response)
        })
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use crate::driver::test_support::*;

    #[tokio::test(flavor = "current_thread")]
    async fn url_errors_match_at_every_public_entry_point() {
        let long_label = "a".repeat(64);
        let long_host = format!("{}.org", "a".repeat(250));
        let cases = [
            (String::new(), crate::InvalidUrl::Empty),
            ("irc.oftc.net".into(), crate::InvalidUrl::MissingScheme),
            (
                "ftp://irc.oftc.net".into(),
                crate::InvalidUrl::UnsupportedScheme("ftp".into()),
            ),
            ("irc://".into(), crate::InvalidUrl::EmptyHost),
            (
                "irc://alice@irc.oftc.net".into(),
                crate::InvalidUrl::Userinfo,
            ),
            (
                "irc://irc.oftc.net?network=oftc".into(),
                crate::InvalidUrl::Query,
            ),
            (
                "irc://irc.oftc.net#support".into(),
                crate::InvalidUrl::Fragment,
            ),
            ("irc://irc oftc.net".into(), crate::InvalidUrl::Whitespace),
            (
                "irc://irc.oftc.net\u{7f}".into(),
                crate::InvalidUrl::ControlCharacter,
            ),
            (
                "irc://irc%2Eoftc.net".into(),
                crate::InvalidUrl::PercentEncoding,
            ),
            (
                "irc://irc.réseau.example".into(),
                crate::InvalidUrl::NonAsciiHost("irc.réseau.example".into()),
            ),
            (
                "irc://irc.oftc.net.".into(),
                crate::InvalidUrl::TrailingDot("irc.oftc.net.".into()),
            ),
            (
                "irc://[fe80::1%25eth0]".into(),
                crate::InvalidUrl::ZoneIdentifier("fe80::1%25eth0".into()),
            ),
            (
                "irc://2001:db8::6697".into(),
                crate::InvalidUrl::UnbracketedIpv6("2001:db8::6697".into()),
            ),
            (
                format!("irc://{long_label}.example"),
                crate::InvalidUrl::InvalidHost(format!("{long_label}.example")),
            ),
            (
                format!("irc://{long_host}"),
                crate::InvalidUrl::InvalidHost(long_host),
            ),
            (
                "ircs://irc.oftc.net:70000".into(),
                crate::InvalidUrl::InvalidPort("70000".into()),
            ),
            (
                "ircs://irc.oftc.net/support".into(),
                crate::InvalidUrl::UnexpectedPath("/support".into()),
            ),
        ];
        let (client, _events, _driver) =
            crate::Client::builder("ircs://irc.example.net", "papo-control").build();

        for (input, expected) in cases {
            assert_eq!(input.parse::<crate::Endpoint>(), Err(expected.clone()));

            let (_client, _events, driver) =
                crate::Client::builder(input.clone(), "papo-client").build();
            let Error::InvalidInput(crate::InvalidInput::Url(builder_error)) =
                within(driver).await.unwrap_err()
            else {
                panic!("client builder returned the wrong error for {input:?}");
            };
            assert_eq!(builder_error, expected);

            let Error::InvalidInput(crate::InvalidInput::Url(add_error)) =
                client.bouncer().add(input).await.unwrap_err()
            else {
                panic!("bouncer add returned the wrong error");
            };
            assert_eq!(add_error, expected);
        }

        for (input, scheme) in [
            ("ws://irc.example.net/webirc", "ws"),
            ("wss://irc.example.net/webirc", "wss"),
        ] {
            let Error::InvalidInput(crate::InvalidInput::Url(error)) =
                client.bouncer().add(input).await.unwrap_err()
            else {
                panic!("bouncer add returned the wrong error for {input:?}");
            };
            assert_eq!(error, crate::InvalidUrl::UnsupportedScheme(scheme.into()));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lists_and_caches_bouncer_networks() {
        let settings = config().enable(crate::Feature::Bouncer).wanted_caps([
            "batch",
            "labeled-response",
            "soju.im/bouncer-networks",
        ]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        complete_connect_caps(
            &mut server,
            &mut events,
            "batch labeled-response soju.im/bouncer-networks",
            "batch labeled-response soju.im/bouncer-networks",
        )
        .await;

        let list = tokio::spawn({
            let client = client.clone();
            async move { client.bouncer().networks().await }
        });
        client_line(&mut server, b"@label=0 BOUNCER LISTNETWORKS\r\n").await;
        server_line(&mut server, b"@label=0 BATCH +networks soju.im/bouncer-networks\r\n@batch=networks BOUNCER NETWORK 42 name=Libera\\sChat;host=irc.libera.chat;state=connected\r\nBATCH -networks\r\n").await;
        let networks = within(list).await.unwrap().unwrap();
        assert_eq!(networks.len(), 1);
        assert_eq!(networks[0].name.as_deref(), Some("Libera Chat"));
        assert!(matches!(
            next_event(&mut events).await,
            Some(Event::BouncerNetworkChanged(_))
        ));
        abort_task(driver).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn adds_changes_and_removes_bouncer_networks() {
        let settings = config()
            .enable(crate::Feature::Bouncer)
            .wanted_caps(["labeled-response", "soju.im/bouncer-networks"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        complete_connect_caps(
            &mut server,
            &mut events,
            "labeled-response soju.im/bouncer-networks",
            "labeled-response soju.im/bouncer-networks",
        )
        .await;
        let add = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .bouncer()
                    .add("ircs://irc.oftc.net")
                    .name("OFTC")
                    .nick("alice")
                    .username("alice")
                    .realname("Alice Example")
                    .password(crate::Secret::new("upstream password"))
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=0 BOUNCER ADDNETWORK host=irc.oftc.net;name=OFTC;port=6697;tls=1;nickname=alice;username=alice;realname=Alice\\sExample;pass=upstream\\spassword\r\n",
        )
        .await;
        server_line(&mut server, b"@label=0 BOUNCER ADDNETWORK 44\r\n").await;
        let id = within(add).await.unwrap().unwrap();
        assert_eq!(id, "44");

        let change = tokio::spawn({
            let client = client.clone();
            let id = id.clone();
            async move { client.bouncer().change(&id).name("OFTC Chat").await }
        });
        client_line(
            &mut server,
            b"@label=1 BOUNCER CHANGENETWORK 44 name=OFTC\\sChat\r\n",
        )
        .await;
        server_line(&mut server, b"@label=1 BOUNCER CHANGENETWORK 44\r\n").await;
        assert!(within(change).await.unwrap().is_ok());

        let remove = tokio::spawn({
            let client = client.clone();
            let id = id.clone();
            async move { client.bouncer().remove(&id).await }
        });
        client_line(&mut server, b"@label=2 BOUNCER DELNETWORK 44\r\n").await;
        server_line(&mut server, b"@label=2 BOUNCER DELNETWORK 44\r\n").await;
        assert!(within(remove).await.unwrap().is_ok());

        let add = tokio::spawn({
            let client = client.clone();
            async move { client.bouncer().add("irc://irc.libera.chat").await }
        });
        client_line(
            &mut server,
            b"@label=3 BOUNCER ADDNETWORK host=irc.libera.chat;port=6667;tls=0\r\n",
        )
        .await;
        server_line(&mut server, b"@label=3 NOTICE ADDNETWORK 45\r\n").await;
        assert!(matches!(
            within(add).await.unwrap(),
            Err(Error::UnexpectedReply { command }) if command == "BOUNCER"
        ));

        let add = tokio::spawn(async move { client.bouncer().add("irc://irc.libera.chat").await });
        client_line(
            &mut server,
            b"@label=4 BOUNCER ADDNETWORK host=irc.libera.chat;port=6667;tls=0\r\n",
        )
        .await;
        server_line(&mut server, b"@label=4 BOUNCER ADDNETWORK\r\n").await;
        assert!(matches!(
            within(add).await.unwrap(),
            Err(Error::UnexpectedReply { command }) if command == "BOUNCER"
        ));
        abort_task(driver).await;
    }
}
