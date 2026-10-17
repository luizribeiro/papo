use std::collections::HashMap;

use papo_core::{MetadataCommand, MetadataValue};

use crate::{Client, Error, driver::CoreRequest};

#[cfg(test)]
use crate::driver::test_support::*;

/// Handle for metadata requests and cached metadata state.
pub struct Metadata {
    client: Client,
}

impl Client {
    /// Returns a handle for metadata requests and cached state.
    #[must_use]
    pub fn metadata(&self) -> Metadata {
        Metadata {
            client: self.clone(),
        }
    }
}

impl Metadata {
    async fn request(&self, command: MetadataCommand) -> Result<(), Error> {
        let response = self
            .client
            .core_request(CoreRequest::Metadata(command))
            .await?;
        response
            .error()
            .map_or(Ok(()), |error| Err(Error::server(error)))
    }

    fn action(&self, command: MetadataCommand) -> crate::RequestFuture<()> {
        let metadata = Self {
            client: self.client.clone(),
        };
        crate::RequestFuture::new(async move { metadata.request(command).await })
    }

    /// Gets selected keys and returns the target's resulting cache.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Metadata`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. Invalid keys return
    /// [`Error::InvalidInput`]; a refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. Request limits, timeouts, and disconnects retain their matching
    /// [`Error`] variants.
    pub fn get<I, S>(
        &self,
        target: impl Into<crate::Target>,
        keys: I,
    ) -> crate::RequestFuture<HashMap<String, MetadataValue>>
    where
        I: IntoIterator<Item = S>,
        S: Into<crate::MetadataKey>,
    {
        let metadata = Self {
            client: self.client.clone(),
        };
        let target = target.into();
        let keys = keys.into_iter().map(Into::into).collect::<Vec<_>>();
        crate::RequestFuture::new(async move {
            let state = metadata.client.state();
            target
                .check(state.isupport())
                .map_err(|error| crate::InvalidInput::InvalidMessage(error.to_string()))?;
            for key in &keys {
                key.check()
                    .map_err(|error| crate::InvalidInput::InvalidMessage(error.to_string()))?;
            }
            metadata
                .request(MetadataCommand::Get {
                    target: target.to_string(),
                    keys: keys
                        .into_iter()
                        .map(crate::MetadataKey::into_string)
                        .collect(),
                })
                .await?;
            Ok(metadata
                .client
                .state()
                .metadata(&target)
                .cloned()
                .unwrap_or_default())
        })
    }

    /// Lists visible keys and returns the target's resulting cache.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Metadata`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. A refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. Request limits, timeouts, and disconnects retain their matching
    /// [`Error`] variants.
    pub fn list(
        &self,
        target: impl Into<crate::Target>,
    ) -> crate::RequestFuture<HashMap<String, MetadataValue>> {
        let metadata = Self {
            client: self.client.clone(),
        };
        let target = target.into();
        crate::RequestFuture::new(async move {
            target
                .check(metadata.client.state().isupport())
                .map_err(|error| crate::InvalidInput::InvalidMessage(error.to_string()))?;
            metadata
                .request(MetadataCommand::List {
                    target: target.to_string(),
                })
                .await?;
            Ok(metadata
                .client
                .state()
                .metadata(&target)
                .cloned()
                .unwrap_or_default())
        })
    }

    /// Sets one metadata key for the current nickname.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Metadata`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. Invalid keys or oversized values return
    /// [`Error::InvalidInput`]; a refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. Request limits, timeouts, and disconnects retain their matching
    /// [`Error`] variants.
    pub fn set(
        &self,
        key: impl Into<crate::MetadataKey>,
        value: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let target = self.client.nick();
        self.set_on(target, key, value)
    }

    /// Removes one metadata key for the current nickname.
    pub fn remove(&self, key: impl Into<crate::MetadataKey>) -> crate::RequestFuture<()> {
        let target = self.client.nick();
        self.remove_on(target, key)
    }

    /// Sets one metadata key on `target`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Metadata`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. Invalid keys or oversized values return
    /// [`Error::InvalidInput`]; a refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. Request limits, timeouts, and disconnects retain their matching
    /// [`Error`] variants.
    pub fn set_on(
        &self,
        target: impl Into<crate::Target>,
        key: impl Into<crate::MetadataKey>,
        value: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        self.action(MetadataCommand::Set {
            target: target.into().into_string(),
            key: key.into().into_string(),
            value: Some(value.into()),
        })
    }

    /// Removes one metadata key from `target`.
    pub fn remove_on(
        &self,
        target: impl Into<crate::Target>,
        key: impl Into<crate::MetadataKey>,
    ) -> crate::RequestFuture<()> {
        self.action(MetadataCommand::Set {
            target: target.into().into_string(),
            key: key.into().into_string(),
            value: None,
        })
    }

    /// Clears all metadata for the current nickname.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Metadata`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. A refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. Request limits, timeouts, and disconnects retain their matching
    /// [`Error`] variants.
    pub fn clear(&self) -> crate::RequestFuture<()> {
        self.clear_on(self.client.nick())
    }

    /// Clears all metadata on `target`.
    pub fn clear_on(&self, target: impl Into<crate::Target>) -> crate::RequestFuture<()> {
        self.action(MetadataCommand::Clear {
            target: target.into().into_string(),
        })
    }

    /// Subscribes to metadata keys.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Metadata`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. Invalid keys or too many subscriptions
    /// return [`Error::InvalidInput`]; a refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. Request limits, timeouts, and disconnects retain their matching
    /// [`Error`] variants.
    pub fn subscribe<I, S>(&self, keys: I) -> crate::RequestFuture<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<crate::MetadataKey>,
    {
        self.action(MetadataCommand::Sub {
            keys: keys
                .into_iter()
                .map(Into::into)
                .map(crate::MetadataKey::into_string)
                .collect(),
        })
    }

    /// Unsubscribes from metadata keys.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Metadata`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. Invalid keys return
    /// [`Error::InvalidInput`]; a refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. Request limits, timeouts, and disconnects retain their matching
    /// [`Error`] variants.
    pub fn unsubscribe<I, S>(&self, keys: I) -> crate::RequestFuture<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<crate::MetadataKey>,
    {
        self.action(MetadataCommand::Unsub {
            keys: keys
                .into_iter()
                .map(Into::into)
                .map(crate::MetadataKey::into_string)
                .collect(),
        })
    }

    /// Synchronizes cached metadata for a target.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Metadata`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. A refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. Request limits, timeouts, and disconnects retain their matching
    /// [`Error`] variants.
    pub fn sync(&self, target: impl Into<crate::Target>) -> crate::RequestFuture<()> {
        let metadata = Self {
            client: self.client.clone(),
        };
        let target = target.into();
        crate::RequestFuture::new(async move {
            target
                .check(metadata.client.state().isupport())
                .map_err(|error| crate::InvalidInput::InvalidMessage(error.to_string()))?;
            metadata
                .request(MetadataCommand::Sync {
                    target: target.into_string(),
                })
                .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn metadata_reads_writes_and_updates_state() {
        let settings = config().wanted_caps(["batch", "draft/metadata-2", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        complete_caps_registration(
            &mut server,
            &mut events,
            "batch draft/metadata-2=max-subs=100,max-keys=100 labeled-response",
            "batch draft/metadata-2 labeled-response",
        )
        .await;

        let get = tokio::spawn({
            let client = client.clone();
            async move { client.metadata().get("alice", ["avatar"]).await }
        });
        client_line(&mut server, b"@label=0 METADATA alice GET avatar\r\n").await;
        server_line(
            &mut server,
            b"@label=0 :ergo.test BATCH +get labeled-response\r\n@batch=get :ergo.test 761 papo-driver alice avatar * :https://cdn.example.net/alice.png\r\n:ergo.test BATCH -get\r\n",
        )
        .await;
        let metadata = within(get).await.unwrap().unwrap();
        assert_eq!(
            metadata["avatar"].value,
            "https://cdn.example.net/alice.png"
        );
        assert_eq!(
            client.state().metadata("ALICE").unwrap()["avatar"].visibility,
            "*"
        );

        let list = tokio::spawn({
            let client = client.clone();
            async move { client.metadata().list("alice").await }
        });
        client_line(&mut server, b"@label=1 METADATA alice LIST\r\n").await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test BATCH +list labeled-response\r\n@batch=list :ergo.test 761 papo-driver alice pronouns * :they/them\r\n:ergo.test BATCH -list\r\n",
        )
        .await;
        let metadata = within(list).await.unwrap().unwrap();
        assert_eq!(metadata["pronouns"].value, "they/them");

        let set = tokio::spawn({
            let client = client.clone();
            async move { client.metadata().set("status", "testing").await }
        });
        client_line(
            &mut server,
            b"@label=2 METADATA papo-driver SET status testing\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=2 :ergo.test 761 papo-driver papo-driver status * testing\r\n",
        )
        .await;
        assert!(within(set).await.unwrap().is_ok());

        let set_on = tokio::spawn(async move {
            client
                .metadata()
                .set_on("bob", "timezone", "America/New_York")
                .await
        });
        client_line(
            &mut server,
            b"@label=3 METADATA bob SET timezone America/New_York\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=3 :ergo.test 761 papo-driver bob timezone * America/New_York\r\n",
        )
        .await;
        assert!(within(set_on).await.unwrap().is_ok());
        abort_task(driver).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn metadata_management_updates_subscriptions_and_types_failures() {
        let settings = config().wanted_caps(["draft/metadata-2", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        complete_caps_registration(
            &mut server,
            &mut events,
            "draft/metadata-2=max-subs=100,max-keys=100 labeled-response",
            "draft/metadata-2 labeled-response",
        )
        .await;

        let clear = tokio::spawn({
            let client = client.clone();
            async move { client.metadata().clear().await }
        });
        client_line(&mut server, b"@label=0 METADATA papo-driver CLEAR\r\n").await;
        server_line(&mut server, b"@label=0 :ergo.test ACK\r\n").await;
        assert!(within(clear).await.unwrap().is_ok());

        let subscribe = tokio::spawn({
            let client = client.clone();
            async move { client.metadata().subscribe(["avatar", "pronouns"]).await }
        });
        client_line(&mut server, b"@label=1 METADATA * SUB avatar pronouns\r\n").await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test 770 papo-driver avatar pronouns\r\n",
        )
        .await;
        assert!(within(subscribe).await.unwrap().is_ok());
        assert_eq!(
            client.state().metadata_subscriptions(),
            ["avatar", "pronouns"]
        );
        assert_eq!(
            client.state().metadata_subscriptions(),
            ["avatar", "pronouns"]
        );

        let unsubscribe = tokio::spawn({
            let client = client.clone();
            async move { client.metadata().unsubscribe(["pronouns"]).await }
        });
        client_line(&mut server, b"@label=2 METADATA * UNSUB pronouns\r\n").await;
        server_line(
            &mut server,
            b"@label=2 :ergo.test 771 papo-driver pronouns\r\n",
        )
        .await;
        assert!(within(unsubscribe).await.unwrap().is_ok());
        assert_eq!(client.state().metadata_subscriptions(), ["avatar"]);

        let sync = tokio::spawn({
            let client = client.clone();
            async move { client.metadata().sync("#papo").await }
        });
        client_line(&mut server, b"@label=3 METADATA #papo SYNC\r\n").await;
        server_line(&mut server, b"@label=3 :ergo.test ACK\r\n").await;
        assert!(within(sync).await.unwrap().is_ok());

        let clear = tokio::spawn(async move { client.metadata().clear().await });
        client_line(&mut server, b"@label=4 METADATA papo-driver CLEAR\r\n").await;
        server_line(
            &mut server,
            b"@label=4 :ergo.test FAIL METADATA VALUE_INVALID :Metadata request rejected\r\n",
        )
        .await;
        assert!(matches!(
            within(clear).await.unwrap(),
            Err(Error::Server(error)) if error.code() == "VALUE_INVALID"
        ));
        abort_task(driver).await;
    }
}
