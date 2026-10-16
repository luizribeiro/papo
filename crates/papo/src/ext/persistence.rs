use crate::{Client, Error, driver::CoreRequest};

/// Handle for account persistence requests.
pub struct Persistence<'a> {
    client: &'a Client,
}

impl Client {
    /// Returns a handle for account persistence requests and cached state.
    #[must_use]
    pub const fn persistence(&self) -> Persistence<'_> {
        Persistence { client: self }
    }
}

impl Persistence<'_> {
    /// Requests the server's current persistence status.
    ///
    /// # Errors
    ///
    /// Returns an error when the request is unsupported or interrupted.
    pub async fn get(&self) -> Result<papo_core::PersistenceStatus, Error> {
        self.client
            .core_request(CoreRequest::PersistenceGet)
            .await?;
        self.state().ok_or(Error::Disconnected)
    }

    /// Changes the account persistence preference.
    ///
    /// # Errors
    ///
    /// Returns an error when the request is unsupported or interrupted.
    pub async fn set(
        &self,
        setting: papo_core::PersistenceSetting,
    ) -> Result<papo_core::PersistenceStatus, Error> {
        self.client
            .core_request(CoreRequest::PersistenceSet(setting))
            .await?;
        self.state().ok_or(Error::Disconnected)
    }

    /// Returns the most recently reported persistence status.
    #[must_use]
    pub fn state(&self) -> Option<papo_core::PersistenceStatus> {
        self.client.state(crate::State::persistence)
    }
}

#[cfg(test)]
mod tests {
    use crate::driver::test_support::*;

    #[tokio::test(flavor = "current_thread")]
    async fn reads_changes_and_caches_persistence() {
        let settings = config().wanted_caps(["draft/persistence", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
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
        assert!(matches!(events.next().await, Some(Event::Registered(_))));

        let get = tokio::spawn({
            let client = client.clone();
            async move { client.persistence().get().await }
        });
        client_line(&mut server, b"@label=0 PERSISTENCE GET\r\n").await;
        server_line(
            &mut server,
            b"@label=0 :ergo.test PERSISTENCE STATUS OFF OFF\r\n",
        )
        .await;
        assert!(!within(get).await.unwrap().unwrap().effective);

        let set = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .persistence()
                    .set(papo_core::PersistenceSetting::On)
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
        assert!(client.persistence().state().unwrap().effective);
        abort_task(driver).await;
    }
}
