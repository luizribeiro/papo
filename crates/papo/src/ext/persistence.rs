use crate::{Client, Error, PersistenceStatus, driver::CoreRequest};

/// Handle for account persistence requests.
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub struct Persistence {
    client: Client,
}

impl Client {
    /// Returns a handle for account persistence requests and cached state.
    #[must_use]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub fn persistence(&self) -> Persistence {
        Persistence {
            client: self.clone(),
        }
    }
}

impl Persistence {
    /// Requests the server's current persistence status.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Persistence`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. A refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. It can also return [`Error::TooManyPendingRequests`], [`Error::Timeout`],
    /// or [`Error::Disconnected`].
    pub fn get(&self) -> crate::RequestFuture<PersistenceStatus> {
        let client = self.client.clone();
        crate::RequestFuture::new(async move {
            let response = client.core_request(CoreRequest::PersistenceGet).await?;
            crate::requests::empty(&response)?;
            client
                .state()
                .persistence()
                .ok_or_else(Error::connection_lost)
        })
    }

    /// Changes the account persistence preference.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Persistence`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. A refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. It can also return [`Error::TooManyPendingRequests`], [`Error::Timeout`],
    /// or [`Error::Disconnected`].
    pub fn set(
        &self,
        setting: crate::PersistenceSetting,
    ) -> crate::RequestFuture<PersistenceStatus> {
        let client = self.client.clone();
        crate::RequestFuture::new(async move {
            let response = client
                .core_request(CoreRequest::PersistenceSet(setting))
                .await?;
            crate::requests::empty(&response)?;
            client
                .state()
                .persistence()
                .ok_or_else(Error::connection_lost)
        })
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use crate::driver::test_support::*;

    #[tokio::test(flavor = "current_thread")]
    async fn reads_changes_and_caches_persistence() {
        let settings = config().wanted_caps(["draft/persistence", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        welcome_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test CAP * LS :draft/persistence labeled-response\r\n",
        )
        .await;
        client_line(
            &mut server,
            b"CAP REQ :draft/persistence labeled-response\r\n",
        )
        .await;
        server_line(
            &mut server,
            b":ergo.test CAP * ACK :draft/persistence labeled-response\r\n",
        )
        .await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(&mut server, b":ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Connected(_))));

        let get = tokio::spawn({
            let client = client.clone();
            async move { client.persistence().get().await }
        });
        client_line(&mut server, b"@label=0 PERSISTENCE GET\r\n").await;
        server_line(
            &mut server,
            b"@label=0 :ergo.test PERSISTENCE STATUS DEFAULT ON\r\n",
        )
        .await;
        assert!(matches!(
            events.next().await,
            Some(Event::PersistenceChanged(status))
                if status.setting == crate::PersistenceSetting::Default && status.effective
        ));
        let status = within(get).await.unwrap().unwrap();
        assert_eq!(status.setting, crate::PersistenceSetting::Default);
        assert!(status.effective);

        let set = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .persistence()
                    .set(crate::PersistenceSetting::On)
                    .await
            }
        });
        client_line(&mut server, b"@label=1 PERSISTENCE SET ON\r\n").await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test PERSISTENCE STATUS ON ON\r\n",
        )
        .await;
        assert!(within(set).await.unwrap().unwrap().effective);
        assert!(client.state().persistence().unwrap().effective);

        let get = tokio::spawn({
            let client = client.clone();
            async move { client.persistence().get().await }
        });
        client_line(&mut server, b"@label=2 PERSISTENCE GET\r\n").await;
        server_line(
            &mut server,
            b"@label=2 :ergo.test FAIL PERSISTENCE NEEDS_ACCOUNT :An account is required\r\n",
        )
        .await;
        assert!(matches!(
            within(get).await.unwrap(),
            Err(Error::Server(error)) if error.code() == "NEEDS_ACCOUNT"
        ));
        abort_task(driver).await;
    }
}
