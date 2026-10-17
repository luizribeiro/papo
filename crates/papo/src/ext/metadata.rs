use std::collections::HashMap;

use papo_core::{MetadataCommand, MetadataValue};

use crate::{Client, Error, driver::CoreRequest};

#[cfg(test)]
use crate::driver::test_support::*;

/// Handle for metadata requests and cached metadata state.
pub struct Metadata<'a> {
    client: &'a Client,
}

impl Client {
    /// Returns a handle for metadata requests and cached state.
    #[must_use]
    pub const fn metadata(&self) -> Metadata<'_> {
        Metadata { client: self }
    }
}

impl Metadata<'_> {
    async fn request(&self, command: MetadataCommand) -> Result<(), Error> {
        let response = self
            .client
            .core_request(CoreRequest::Metadata(command))
            .await?;
        response
            .error()
            .map_or(Ok(()), |error| Err(Error::server(error)))
    }

    /// Gets selected keys and returns the target's resulting cache.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn get<I, S>(
        &self,
        target: &str,
        keys: I,
    ) -> Result<HashMap<String, MetadataValue>, Error>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.request(MetadataCommand::Get {
            target: target.into(),
            keys: keys.into_iter().map(Into::into).collect(),
        })
        .await?;
        Ok(self
            .client
            .state(|state| state.metadata(target).cloned().unwrap_or_default()))
    }

    /// Lists visible keys and returns the target's resulting cache.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn list(&self, target: &str) -> Result<HashMap<String, MetadataValue>, Error> {
        self.request(MetadataCommand::List {
            target: target.into(),
        })
        .await?;
        Ok(self
            .client
            .state(|state| state.metadata(target).cloned().unwrap_or_default()))
    }

    /// Sets or removes one metadata key for the current nickname.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn set(&self, key: &str, value: Option<&str>) -> Result<(), Error> {
        let target = self.client.nick();
        self.set_on(&target, key, value).await
    }

    /// Sets or removes one metadata key on `target`.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn set_on(&self, target: &str, key: &str, value: Option<&str>) -> Result<(), Error> {
        self.request(MetadataCommand::Set {
            target: target.into(),
            key: key.into(),
            value: value.map(str::to_owned),
        })
        .await
    }

    /// Clears all metadata for the current nickname.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn clear(&self) -> Result<(), Error> {
        self.request(MetadataCommand::Clear {
            target: self.client.nick(),
        })
        .await
    }

    /// Subscribes to metadata keys.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn subscribe<I, S>(&self, keys: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.request(MetadataCommand::Sub {
            keys: keys.into_iter().map(Into::into).collect(),
        })
        .await
    }

    /// Unsubscribes from metadata keys.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn unsubscribe<I, S>(&self, keys: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.request(MetadataCommand::Unsub {
            keys: keys.into_iter().map(Into::into).collect(),
        })
        .await
    }

    /// Returns the cached metadata subscription list.
    #[must_use]
    pub fn subscriptions(&self) -> Vec<String> {
        self.client
            .state(|state| state.metadata_subscriptions().to_vec())
    }

    /// Synchronizes cached metadata for a target.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn sync(&self, target: &str) -> Result<(), Error> {
        self.request(MetadataCommand::Sync {
            target: target.into(),
        })
        .await
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
        client.state(|state| {
            assert_eq!(state.metadata("ALICE").unwrap()["avatar"].visibility, "*");
        });

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
            async move { client.metadata().set("status", Some("testing")).await }
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
                .set_on("bob", "timezone", Some("America/New_York"))
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
        assert_eq!(client.metadata().subscriptions(), ["avatar", "pronouns"]);
        client.state(|state| {
            assert_eq!(state.metadata_subscriptions(), ["avatar", "pronouns"]);
        });

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
        assert_eq!(client.metadata().subscriptions(), ["avatar"]);

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
