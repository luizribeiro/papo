use crate::{BouncerNetwork, Client, driver::CoreRequest};

/// Handle for upstream bouncer network management.
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub struct Bouncer {
    client: Client,
}

impl Client {
    /// Returns a handle for upstream bouncer network management.
    #[must_use]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub fn bouncer(&self) -> Bouncer {
        Bouncer {
            client: self.clone(),
        }
    }
}

impl Bouncer {
    /// Requests and returns the complete upstream network list.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Unsupported`] for [`crate::Feature::Bouncer`] or an error documented by
    /// [`Client::request_raw`].
    pub fn networks(&self) -> crate::RequestFuture<Vec<BouncerNetwork>> {
        let client = self.client.clone();
        crate::RequestFuture::new(async move {
            client.core_request(CoreRequest::BouncerNetworks).await?;
            Ok(client.state().bouncer_networks().cloned().collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::driver::test_support::*;

    #[tokio::test(flavor = "current_thread")]
    async fn lists_and_caches_bouncer_networks() {
        let settings =
            config().wanted_caps(["batch", "labeled-response", "soju.im/bouncer-networks"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        complete_caps_registration(
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
            Some(Event::BouncerNetwork(_))
        ));
        abort_task(driver).await;
    }
}
