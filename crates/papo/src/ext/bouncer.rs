use crate::{
    BouncerNetwork, Client, Error, NetworkConfig, NetworkId, driver::unstable::UnstableRequest,
};

/// Handle for upstream bouncer network management.
///
/// A control connection can discover networks and create separate device connections to one of
/// them. Put [`crate::Builder::device`] on each network-bound builder returned by [`Self::network`],
/// rather than on the control connection. Network names need not be unique, so applications that
/// select by name must define how to resolve multiple matches.
///
/// ```no_run
/// use papo::{Client, Feature, Sasl};
///
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let (control, _control_events) = Client::builder("ircs://bouncer.example.net")
///     .nick("alice")
///     .sasl(Sasl::plain("alice", "correct horse battery staple"))
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
///     .device("phone")
///     .autojoin(["#papo"])
///     .connect()
///     .await?;
/// let (_laptop, _laptop_events) = control
///     .bouncer()
///     .network(&network.id)
///     .device("laptop")
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
    /// the control connection unless [`crate::Builder::request_caps`] explicitly requests them.
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
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Bouncer`], [`Error::Server`] when
    /// refused, or an error documented by [`Client::request_raw`].
    pub fn add(&self, config: NetworkConfig) -> crate::RequestFuture<NetworkId> {
        let client = self.client.clone();
        crate::RequestFuture::new(async move {
            let response = client
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
                .ok_or_else(|| Error::unexpected_reply("BOUNCER ADDNETWORK"))
        })
    }

    /// Replaces the configurable attributes of an upstream network.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Bouncer`], [`Error::Server`] when
    /// refused, or an error documented by [`Client::request_raw`].
    pub fn change(&self, id: &NetworkId, config: NetworkConfig) -> crate::RequestFuture<()> {
        let client = self.client.clone();
        let id = id.clone();
        crate::RequestFuture::new(async move {
            let response = client
                .core_request(UnstableRequest::BouncerChange(id, config))
                .await?;
            crate::requests::empty(&response)
        })
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
    use crate::NetworkConfig;
    use crate::driver::test_support::*;

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
            events.next().await,
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
        let config = NetworkConfig::new("irc.oftc.net").name("OFTC");
        let add = tokio::spawn({
            let client = client.clone();
            async move { client.bouncer().add(config).await }
        });
        client_line(
            &mut server,
            b"@label=0 BOUNCER ADDNETWORK host=irc.oftc.net;name=OFTC\r\n",
        )
        .await;
        server_line(&mut server, b"@label=0 BOUNCER ADDNETWORK 44\r\n").await;
        let id = within(add).await.unwrap().unwrap();
        assert_eq!(id, "44");

        let change = tokio::spawn({
            let client = client.clone();
            let id = id.clone();
            async move {
                client
                    .bouncer()
                    .change(&id, NetworkConfig::new("irc.oftc.net").tls(true))
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=1 BOUNCER CHANGENETWORK 44 host=irc.oftc.net;tls=1\r\n",
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
            async move {
                client
                    .bouncer()
                    .add(NetworkConfig::new("irc.libera.chat"))
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=3 BOUNCER ADDNETWORK host=irc.libera.chat\r\n",
        )
        .await;
        server_line(&mut server, b"@label=3 NOTICE ADDNETWORK 45\r\n").await;
        assert!(matches!(
            within(add).await.unwrap(),
            Err(Error::UnexpectedReply { command }) if command == "BOUNCER ADDNETWORK"
        ));

        let add = tokio::spawn(async move {
            client
                .bouncer()
                .add(NetworkConfig::new("irc.libera.chat"))
                .await
        });
        client_line(
            &mut server,
            b"@label=4 BOUNCER ADDNETWORK host=irc.libera.chat\r\n",
        )
        .await;
        server_line(&mut server, b"@label=4 BOUNCER ADDNETWORK\r\n").await;
        assert!(matches!(
            within(add).await.unwrap(),
            Err(Error::UnexpectedReply { command }) if command == "BOUNCER ADDNETWORK"
        ));
        abort_task(driver).await;
    }
}
