use std::{collections::HashMap, future::Future};

use papo_core::MetadataCommand;

use crate::{Client, Error, MetadataValue, driver::CoreRequest};

/// What the server did with each key of a [`Metadata::subscribe`] request.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SubscribeOutcome {
    /// Keys the server now sends updates for, as listed in `RPL_METADATASUBOK`.
    pub subscribed: Vec<crate::MetadataKey>,
    /// Requested keys the server left out, with the refusal of its `FAIL`.
    ///
    /// A server that stops at its limit names only the first key it refused, so every key it
    /// left out carries that refusal, or [`crate::Refusal::Other`] when it sent none.
    pub refused: Vec<(crate::MetadataKey, crate::Refusal)>,
}

/// How the server answered a [`Metadata::sync`] request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SyncOutcome {
    /// The server answered without deferring; any metadata it sent is now in state.
    Synced,
    /// The server deferred the sync with `RPL_METADATASYNCLATER`.
    #[non_exhaustive]
    Later {
        /// How long the server asked to wait before syncing again, when it said.
        retry_after: Option<std::time::Duration>,
    },
}

/// Handle for metadata requests and cached metadata state.
///
/// Requests require negotiated metadata and `labeled-response`. Awaiting returns
/// [`Error::Unsupported`] with the missing feature before sending the request.
pub struct Metadata {
    client: Client,
}

impl std::fmt::Debug for Metadata {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("Metadata", formatter)
    }
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
    fn target(
        &self,
        target: impl crate::AsConversation,
        argument: &'static str,
    ) -> (Result<String, Error>, Option<crate::UserRef>) {
        let parts = crate::AsConversation::__parts(&target).into_owned();
        let user = parts.user.clone();
        let target = self
            .client
            .resolve(crate::target::Role::Conversation(argument), parts);
        (target, user)
    }

    async fn request(
        &self,
        command: MetadataCommand,
        proof: Option<papo_core::Proven>,
    ) -> Result<papo_core::Response, Error> {
        let request = match proof {
            Some(proof) => CoreRequest::MetadataPerson { command, proof },
            None => CoreRequest::Metadata(command),
        };
        let response = self.client.core_request(request).await?;
        crate::requests::empty(&response)?;
        Ok(response)
    }

    fn prove_target(
        &self,
        target: &str,
        user: Option<crate::UserRef>,
        argument: &'static str,
    ) -> Result<Option<papo_core::Proven>, Error> {
        if target == "*" || self.client.state().isupport().is_channel(target) {
            return Ok(None);
        }
        let parts = user.map_or_else(
            || crate::AsNick::__parts(&target.to_owned()).into_owned(),
            |user| crate::AsNick::__parts(&user).into_owned(),
        );
        Ok(Some(
            crate::identity::prove_parts(&self.client, parts, argument)?.core,
        ))
    }

    fn action(&self, command: MetadataCommand) -> crate::RequestFuture<()> {
        let metadata = Self {
            client: self.client.clone(),
        };
        crate::RequestFuture::new(async move { metadata.request(command, None).await.map(drop) })
    }

    fn set_target(
        &self,
        target: Result<String, Error>,
        user: Option<crate::UserRef>,
        key: crate::MetadataKey,
        value: Option<String>,
    ) -> impl Future<Output = Result<papo_core::Response, Error>> + Send + 'static {
        let metadata = Self {
            client: self.client.clone(),
        };
        let key = key.into_string();
        async move {
            let target = target?;
            if value.as_ref().is_some_and(String::is_empty) {
                return Err(crate::InvalidInput::EmptyText.into());
            }
            let proof = metadata.prove_target(&target, user, "metadata: target")?;
            metadata
                .request(MetadataCommand::__set(target, key, value), proof)
                .await
        }
    }

    fn store(
        &self,
        target: Result<String, Error>,
        user: Option<crate::UserRef>,
        key: crate::MetadataKey,
        value: String,
    ) -> crate::RequestFuture<MetadataValue> {
        let response = self.set_target(target, user, key.clone(), Some(value));
        crate::RequestFuture::new(async move {
            response
                .await?
                .metadata()
                .into_iter()
                .find_map(|(stored, value)| (stored == key).then_some(value).flatten())
                .ok_or(Error::NotVerified)
        })
    }

    fn unset(
        &self,
        target: Result<String, Error>,
        user: Option<crate::UserRef>,
        key: crate::MetadataKey,
    ) -> crate::RequestFuture<()> {
        let response = self.set_target(target, user, key, None);
        crate::RequestFuture::new(async move { response.await.map(drop) })
    }

    fn clear_target(
        &self,
        target: Result<String, Error>,
        user: Option<crate::UserRef>,
    ) -> crate::RequestFuture<Vec<crate::MetadataKey>> {
        let metadata = Self {
            client: self.client.clone(),
        };
        crate::RequestFuture::new(async move {
            let target = target?;
            let proof = metadata.prove_target(&target, user, "metadata: target")?;
            let response = metadata
                .request(MetadataCommand::__clear(target), proof)
                .await?;
            Ok(response
                .metadata()
                .into_iter()
                .filter_map(|(key, value)| value.is_none().then_some(key))
                .collect())
        })
    }

    /// Gets selected keys and returns only their resulting values.
    ///
    /// A key whose value is `None` was reported by the server as not set.
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
        target: impl crate::AsConversation,
        keys: I,
    ) -> crate::RequestFuture<HashMap<crate::MetadataKey, Option<MetadataValue>>>
    where
        I: IntoIterator<Item = S>,
        S: Into<crate::MetadataKey>,
    {
        let metadata = Self {
            client: self.client.clone(),
        };
        let (target, user) = self.target(target, "metadata.get: target");
        let keys = keys.into_iter().map(Into::into).collect::<Vec<_>>();
        crate::RequestFuture::new(async move {
            let target = target?;
            let state = metadata.client.state();
            state.parse_target_inner(&target).map_err(|reason| {
                crate::InvalidInput::invalid_argument(
                    "metadata.get: target",
                    target.clone(),
                    reason,
                )
            })?;
            for key in &keys {
                key.check()
                    .map_err(|reason| crate::InvalidInput::InvalidMetadataKey {
                        key: key.to_string(),
                        reason,
                    })?;
            }
            let proof = metadata.prove_target(&target, user, "metadata.get: target")?;
            metadata
                .request(
                    MetadataCommand::__get(
                        target.clone(),
                        keys.iter()
                            .cloned()
                            .map(crate::MetadataKey::into_string)
                            .collect(),
                    ),
                    proof,
                )
                .await?;
            let state = metadata.client.state();
            let values = state.metadata(&target);
            Ok(keys
                .into_iter()
                .map(|key| {
                    let value = values.and_then(|values| values.get(&key)).cloned();
                    (key, value)
                })
                .collect())
        })
    }

    /// Lists visible keys and returns the user or channel's resulting cache.
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
        target: impl crate::AsConversation,
    ) -> crate::RequestFuture<HashMap<crate::MetadataKey, MetadataValue>> {
        let metadata = Self {
            client: self.client.clone(),
        };
        let (target, user) = self.target(target, "metadata.list: target");
        crate::RequestFuture::new(async move {
            let target = target?;
            let proof = metadata.prove_target(&target, user, "metadata.list: target")?;
            metadata
                .request(MetadataCommand::__list(target.clone()), proof)
                .await?;
            Ok(metadata
                .client
                .state()
                .metadata(&target)
                .map(|values| {
                    values
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect()
                })
                .unwrap_or_default())
        })
    }

    /// Sets one metadata key for the current nickname and resolves to the value the server
    /// stored, which may differ from the one sent.
    ///
    /// An empty value is invalid input. Use [`Metadata::remove`] to remove the key.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotVerified`] when the server answers without the stored value.
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Metadata`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. Invalid keys or oversized values return
    /// [`Error::InvalidInput`]; creating a target or key past its configured bound returns
    /// [`Error::StateLimit`] before sending; a refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. Request limits, timeouts, and disconnects retain their matching
    /// [`Error`] variants.
    pub fn set(
        &self,
        key: crate::MetadataKey,
        value: impl Into<String>,
    ) -> crate::RequestFuture<MetadataValue> {
        self.store(Ok("*".into()), None, key, value.into())
    }

    /// Removes one metadata key for the current nickname.
    ///
    /// The server's `RPL_KEYNOTSET` repeats only the key, so this resolves to `()`.
    pub fn remove(&self, key: crate::MetadataKey) -> crate::RequestFuture<()> {
        self.unset(Ok("*".into()), None, key)
    }

    /// Sets one metadata key on a user or channel and resolves to the value the server stored.
    ///
    /// An empty value is invalid input. Use [`Metadata::remove_on`] to remove the key.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotVerified`] when the server answers without the stored value.
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Metadata`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. Invalid keys or oversized values return
    /// [`Error::InvalidInput`]; creating a target or key past its configured bound returns
    /// [`Error::StateLimit`] before sending; a refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. Request limits, timeouts, and disconnects retain their matching
    /// [`Error`] variants.
    pub fn set_on(
        &self,
        target: impl crate::AsConversation,
        key: crate::MetadataKey,
        value: impl Into<String>,
    ) -> crate::RequestFuture<MetadataValue> {
        let (target, user) = self.target(target, "metadata.set_on: target");
        self.store(target, user, key, value.into())
    }

    /// Removes one metadata key from a user or channel.
    ///
    /// The server's `RPL_KEYNOTSET` repeats only the key, so this resolves to `()`.
    pub fn remove_on(
        &self,
        target: impl crate::AsConversation,
        key: crate::MetadataKey,
    ) -> crate::RequestFuture<()> {
        let (target, user) = self.target(target, "metadata.remove_on: target");
        self.unset(target, user, key)
    }

    /// Clears all metadata for the current nickname and resolves to the keys the server cleared.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Metadata`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. A refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. Request limits, timeouts, and disconnects retain their matching
    /// [`Error`] variants.
    pub fn clear(&self) -> crate::RequestFuture<Vec<crate::MetadataKey>> {
        self.clear_target(Ok("*".into()), None)
    }

    /// Clears all metadata on a user or channel and resolves to the keys the server cleared.
    pub fn clear_on(
        &self,
        target: impl crate::AsConversation,
    ) -> crate::RequestFuture<Vec<crate::MetadataKey>> {
        let (target, user) = self.target(target, "metadata.clear_on: target");
        self.clear_target(target, user)
    }

    /// Subscribes to metadata keys and resolves to the keys the server subscribed and refused.
    ///
    /// A server may refuse some keys, for example past its subscription limit, while it
    /// subscribes the others; the request succeeds when any key was subscribed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Metadata`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. Invalid keys or too many subscriptions
    /// return [`Error::InvalidInput`]; a refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. Request limits, timeouts, and disconnects retain their matching
    /// [`Error`] variants.
    pub fn subscribe<I, S>(&self, keys: I) -> crate::RequestFuture<SubscribeOutcome>
    where
        I: IntoIterator<Item = S>,
        S: Into<crate::MetadataKey>,
    {
        let client = self.client.clone();
        let requested: Vec<crate::MetadataKey> = keys.into_iter().map(Into::into).collect();
        let command = MetadataCommand::__sub(
            requested
                .iter()
                .cloned()
                .map(crate::MetadataKey::into_string)
                .collect(),
        );
        crate::RequestFuture::new(async move {
            let response = client.core_request(CoreRequest::Metadata(command)).await?;
            let subscribed: Vec<_> = subscribed(&response).collect();
            if subscribed.is_empty() {
                crate::requests::empty(&response)?;
            }
            let refusal = response.refusal().unwrap_or(crate::Refusal::Other);
            let refused = requested
                .into_iter()
                .filter(|key| !subscribed.contains(key))
                .map(|key| (key, refusal))
                .collect();
            Ok(SubscribeOutcome {
                subscribed,
                refused,
            })
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
        self.action(MetadataCommand::__unsub(
            keys.into_iter()
                .map(Into::into)
                .map(crate::MetadataKey::into_string)
                .collect(),
        ))
    }

    /// Synchronizes cached metadata for a user or channel and resolves to whether the server
    /// synced now or asked to retry later.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::Metadata`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. A refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. Request limits, timeouts, and disconnects retain their matching
    /// [`Error`] variants.
    pub fn sync(&self, target: impl crate::AsConversation) -> crate::RequestFuture<SyncOutcome> {
        let metadata = Self {
            client: self.client.clone(),
        };
        let (target, user) = self.target(target, "metadata.sync: target");
        crate::RequestFuture::new(async move {
            let target = target?;
            let proof = metadata.prove_target(&target, user, "metadata.sync: target")?;
            let response = metadata
                .request(MetadataCommand::__sync(target), proof)
                .await?;
            Ok(match response.sync_later() {
                Some(retry_after) => SyncOutcome::Later { retry_after },
                None => SyncOutcome::Synced,
            })
        })
    }
}

fn subscribed(response: &papo_core::Response) -> impl Iterator<Item = crate::MetadataKey> + '_ {
    response
        .lines
        .iter()
        .map(papo_proto::Line::as_ref)
        .filter(|line| {
            line.command() == papo_proto::Command::Numeric(papo_proto::Numeric::RPL_METADATASUBOK)
        })
        .flat_map(|line| {
            line.params()
                .skip(1)
                .map(crate::MetadataKey::new)
                .collect::<Vec<_>>()
        })
}

#[cfg(all(test, not(browser)))]
mod tests {
    use crate::driver::test_support::*;

    #[tokio::test(flavor = "current_thread")]
    async fn metadata_person_target_refuses_a_nickname_handoff() {
        let settings = config().wanted_caps(["draft/metadata-2", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        complete_connect_caps(
            &mut server,
            &mut events,
            "draft/metadata-2=max-subs=100,max-keys=100 labeled-response",
            "draft/metadata-2 labeled-response",
        )
        .await;
        server_line(&mut server, b":papo-driver!p@localhost JOIN #review\r\n").await;
        _ = next_event(&mut events).await;
        server_line(&mut server, b":alice!a@alice.example JOIN #review\r\n").await;
        let alice = match next_event(&mut events).await {
            Some(crate::Event::Joined(joined)) => joined.actor,
            event => panic!("expected Alice's join, got {event:?}"),
        };
        server_line(&mut server, b":mallory!m@mallory.example JOIN #review\r\n").await;
        _ = next_event(&mut events).await;
        server_line(
            &mut server,
            b":alice!a@alice.example QUIT :Client Quit\r\n:mallory!m@mallory.example NICK alice\r\n",
        )
        .await;
        _ = next_event(&mut events).await;
        _ = next_event(&mut events).await;

        assert!(matches!(
            client.metadata().list(&alice).await,
            Err(crate::Error::TargetGone)
        ));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), server.read_u8())
                .await
                .is_err()
        );
        abort_task(driver).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn metadata_enforces_argument_roles() {
        let (first, _, _) = crate::Client::builder("ircs://irc.example.net", "papo-client")
            .casemapping(crate::Casemapping::default())
            .build();
        let (second, _, _) = crate::Client::builder("ircs://irc.example.net", "papo-client")
            .casemapping(crate::Casemapping::default())
            .build();
        let channel = first
            .state()
            .name_rules()
            .expect("server advertised CASEMAPPING")
            .channel("#papo");

        assert!(matches!(
            second.metadata().clear_on(channel).await,
            Err(crate::Error::ForeignReference(_))
        ));
        assert!(matches!(
            second.metadata().clear_on("@#papo").await,
            Err(crate::Error::InvalidInput(
                crate::InvalidInput::InvalidName {
                    argument: "metadata.clear_on: target",
                    ..
                }
            ))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    #[allow(clippy::too_many_lines)]
    async fn metadata_reads_writes_and_updates_state() {
        let settings = config().wanted_caps(["batch", "draft/metadata-2", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        complete_connect_caps(
            &mut server,
            &mut events,
            "batch draft/metadata-2=max-subs=100,max-keys=100 labeled-response",
            "batch draft/metadata-2 labeled-response",
        )
        .await;

        server_line(
            &mut server,
            b":ergo.test 761 papo-driver alice status * :working remotely\r\n",
        )
        .await;
        assert!(matches!(
            next_event(&mut events).await,
            Some(crate::Event::MetadataChanged(changed)) if changed.key == "status"
        ));

        let get = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .metadata()
                    .get(
                        "alice",
                        [
                            crate::MetadataKey::new("avatar"),
                            crate::MetadataKey::new("pronouns"),
                        ],
                    )
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=0 METADATA alice GET avatar pronouns\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=0 :ergo.test BATCH +get labeled-response\r\n@batch=get :ergo.test 761 papo-driver alice avatar * :https://cdn.example.net/alice.png\r\n@batch=get :ergo.test 766 papo-driver alice pronouns :key is not set\r\n:ergo.test BATCH -get\r\n",
        )
        .await;
        let metadata = within(get).await.unwrap().unwrap();
        assert_eq!(
            metadata["avatar"].as_ref().unwrap().value,
            "https://cdn.example.net/alice.png"
        );
        assert_eq!(metadata["pronouns"], None);
        assert_eq!(metadata.len(), 2);
        assert!(!metadata.contains_key("status"));
        assert_eq!(
            client
                .state()
                .metadata("ALICE")
                .unwrap()
                .get(&crate::metadata::keys::AVATAR)
                .unwrap()
                .visibility,
            crate::Visibility::Public
        );

        let list = tokio::spawn({
            let client = client.clone();
            async move { client.metadata().list("alice").await }
        });
        client_line(&mut server, b"@label=1 METADATA alice LIST\r\n").await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test BATCH +list labeled-response\r\n@batch=list :ergo.test 761 papo-driver alice pronouns channel :they/them\r\n:ergo.test BATCH -list\r\n",
        )
        .await;
        let metadata = within(list).await.unwrap().unwrap();
        assert_eq!(metadata["pronouns"].value, "they/them");
        assert_eq!(
            metadata["pronouns"].visibility,
            crate::Visibility::Restricted("channel".to_owned())
        );

        let set = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .metadata()
                    .set(crate::metadata::keys::STATUS, "testing")
                    .await
            }
        });
        client_line(&mut server, b"@label=2 METADATA * SET status testing\r\n").await;
        server_line(
            &mut server,
            b"@label=2 :ergo.test 761 papo-driver papo-driver status * testing\r\n",
        )
        .await;
        assert!(within(set).await.unwrap().is_ok());

        let remove = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .metadata()
                    .remove(crate::MetadataKey::new("status"))
                    .await
            }
        });
        client_line(&mut server, b"@label=3 METADATA * SET status\r\n").await;
        server_line(&mut server, b"@label=3 :ergo.test ACK\r\n").await;
        assert!(within(remove).await.unwrap().is_ok());

        assert!(matches!(
            client
                .metadata()
                .set(crate::MetadataKey::new("status"), "")
                .await,
            Err(Error::InvalidInput(crate::InvalidInput::EmptyText))
        ));
        assert!(matches!(
            client
                .metadata()
                .set_on("bob", crate::MetadataKey::new("timezone"), "")
                .await,
            Err(Error::InvalidInput(crate::InvalidInput::EmptyText))
        ));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1), server.read_u8())
                .await
                .is_err()
        );

        let set_on = tokio::spawn(async move {
            client
                .metadata()
                .set_on(
                    "bob",
                    crate::MetadataKey::new("timezone"),
                    "America/New_York",
                )
                .await
        });
        client_line(
            &mut server,
            b"@label=4 METADATA bob SET timezone America/New_York\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=4 :ergo.test 761 papo-driver bob timezone * America/New_York\r\n",
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
        complete_connect_caps(
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
        client_line(&mut server, b"@label=0 METADATA * CLEAR\r\n").await;
        server_line(&mut server, b"@label=0 :ergo.test ACK\r\n").await;
        assert!(within(clear).await.unwrap().is_ok());

        let subscribe = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .metadata()
                    .subscribe([
                        crate::MetadataKey::new("avatar"),
                        crate::MetadataKey::new("pronouns"),
                    ])
                    .await
            }
        });
        client_line(&mut server, b"@label=1 METADATA * SUB avatar pronouns\r\n").await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test 770 papo-driver avatar pronouns\r\n",
        )
        .await;
        assert!(within(subscribe).await.unwrap().is_ok());
        assert!(
            client
                .state()
                .metadata_subscriptions()
                .map(AsRef::as_ref)
                .eq(["avatar", "pronouns"])
        );

        let unsubscribe = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .metadata()
                    .unsubscribe([crate::MetadataKey::new("pronouns")])
                    .await
            }
        });
        client_line(&mut server, b"@label=2 METADATA * UNSUB pronouns\r\n").await;
        server_line(
            &mut server,
            b"@label=2 :ergo.test 771 papo-driver pronouns\r\n",
        )
        .await;
        assert!(within(unsubscribe).await.unwrap().is_ok());
        assert!(
            client
                .state()
                .metadata_subscriptions()
                .map(AsRef::as_ref)
                .eq(["avatar"])
        );

        let sync = tokio::spawn({
            let client = client.clone();
            async move { client.metadata().sync("#papo").await }
        });
        client_line(&mut server, b"@label=3 METADATA #papo SYNC\r\n").await;
        server_line(&mut server, b"@label=3 :ergo.test ACK\r\n").await;
        assert!(within(sync).await.unwrap().is_ok());

        let clear = tokio::spawn(async move { client.metadata().clear().await });
        client_line(&mut server, b"@label=4 METADATA * CLEAR\r\n").await;
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
