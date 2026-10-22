use std::sync::{Arc, Mutex};

use papo::{Builder, Client, Endpoint, IntoEndpoint, Sasl};
use papo_test_server::{ServerInfo, create_account, unique};

use papo_testing::{Error, Result, skipped};

use crate::Peer;

const ACCOUNT_PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

/// A live-server test scenario that owns every connected peer.
#[derive(Clone)]
pub struct Scenario {
    server: ServerInfo,
    clients: Arc<Mutex<Vec<Client>>>,
}

impl Scenario {
    /// Creates an empty scenario for a runner-provided server.
    #[must_use]
    pub fn new(server: ServerInfo) -> Self {
        Self {
            server,
            clients: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Configures a client with a unique nickname derived from `label`.
    ///
    /// # Errors
    ///
    /// Returns an error when a selected bouncer network cannot be provisioned.
    pub fn configure(&self, label: &str) -> Result<Builder> {
        let address = self.server.plaintext_addr();
        self.configure_named_at(
            unique_name(label),
            Endpoint::plaintext(address.ip().to_string(), address.port()).unwrap(),
        )
    }

    /// Configures a named client for an explicit endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when a selected bouncer network cannot be provisioned.
    pub fn configure_at(&self, label: &str, endpoint: impl IntoEndpoint) -> Result<Builder> {
        self.configure_named_at(unique_name(label), endpoint)
    }

    /// Configures, connects, and retains a peer named from `label`.
    ///
    /// # Errors
    ///
    /// Returns the client's connection failure.
    pub async fn connect(&self, label: &str) -> Result<Peer> {
        self.connect_with(self.configure(label)?).await
    }

    /// Creates an account, authenticates it with SASL PLAIN, and connects it.
    ///
    /// # Errors
    ///
    /// Returns an error when the server cannot create the account or the
    /// authenticated client cannot connect.
    pub async fn connect_as_account(&self, label: &str) -> Result<Peer> {
        Ok(self.connect_as_account_with_key(label).await?.0)
    }

    /// Registers an account and returns a connected peer plus its fixture session key.
    ///
    /// The key can be passed to [`Self::configure_account_session`] to attach more clients,
    /// including when the selected profile uses a bouncer network.
    ///
    /// # Errors
    ///
    /// Returns an error when the server cannot register the account or the
    /// authenticated client cannot connect.
    pub async fn connect_as_account_with_key(&self, label: &str) -> Result<(Peer, String)> {
        let account = unique_name(label);
        create_account(&self.server, &account, ACCOUNT_PASSWORD)?;
        let address = self.server.plaintext_addr();
        let builder = self.configure_named_at(
            account.clone(),
            Endpoint::plaintext(address.ip().to_string(), address.port()).unwrap(),
        )?;
        let builder = if self.server.bouncer().is_some() {
            builder
        } else {
            builder.sasl(Sasl::plain(account.clone(), ACCOUNT_PASSWORD))
        };
        Ok((self.connect_with(builder).await?, account))
    }

    /// Connects another client authenticated to an account created by this fixture.
    ///
    /// # Errors
    ///
    /// Returns the client's connection or authentication failure.
    pub async fn connect_account_session(
        &self,
        label: &str,
        account: impl Into<String>,
    ) -> Result<Peer> {
        self.connect_with(self.configure_account_session(label, account)?)
            .await
    }

    /// Configures another client authenticated to an account created by this fixture.
    ///
    /// # Errors
    ///
    /// Returns an error when a selected bouncer network cannot be addressed.
    pub fn configure_account_session(
        &self,
        label: &str,
        account: impl Into<String>,
    ) -> Result<Builder> {
        let account = account.into();
        let address = self.server.plaintext_addr();
        let builder = Self::builder_named_at(
            unique_name(label),
            Endpoint::plaintext(address.ip().to_string(), address.port()).unwrap(),
        );
        let builder = if let Some(bouncer) = self.server.bouncer() {
            let login = bouncer.network_login(&account)?;
            builder.sasl(Sasl::plain(login.username(), login.password()))
        } else {
            builder.sasl(Sasl::plain(account, ACCOUNT_PASSWORD))
        };
        Ok(builder)
    }

    /// Connects and retains a client configured by the caller.
    ///
    /// # Errors
    ///
    /// Returns the client's connection failure.
    pub async fn connect_with(&self, builder: Builder) -> Result<Peer> {
        let (client, events) = builder.connect().await?;
        self.clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(client.clone());
        Ok(Peer::new(client, events))
    }

    /// Returns a unique, readable channel name.
    #[must_use]
    #[allow(clippy::unused_self)]
    pub fn channel_name(&self) -> String {
        format!("#{}", unique_name("channel"))
    }

    /// Skips a scenario at runtime with the supplied reason.
    ///
    /// # Errors
    ///
    /// Always returns the runner's typed skip marker.
    #[allow(clippy::unused_self)]
    pub fn skip<T>(&self, reason: impl Into<String>) -> Result<T> {
        Err(skipped(reason))
    }

    /// Quits every peer connected through this scenario.
    ///
    /// Scenario attributes call this after the test body, including when the
    /// body returns an error.
    ///
    /// # Errors
    ///
    /// Returns the first failure after attempting to quit every peer. Peers that
    /// already disconnected during the scenario need no further cleanup.
    #[doc(hidden)]
    pub async fn quit_all(&self) -> Result<()> {
        let clients = std::mem::take(
            &mut *self
                .clients
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let mut first: Option<Error> = None;
        for client in clients {
            match client.quit("test scenario complete").await {
                Ok(()) | Err(papo::Error::Disconnected(_)) => {}
                Err(error) if first.is_none() => first = Some(error.into()),
                Err(_) => {}
            }
        }
        first.map_or(Ok(()), Err)
    }

    fn configure_named_at(&self, name: String, endpoint: impl IntoEndpoint) -> Result<Builder> {
        let bouncer_login = self.server.bouncer_login(&name)?;
        let builder = Self::builder_named_at(name, endpoint);
        let builder = match bouncer_login {
            Some(login) => builder.sasl(Sasl::plain(login.username(), login.password())),
            None => builder,
        };
        Ok(builder)
    }

    fn builder_named_at(name: String, endpoint: impl IntoEndpoint) -> Builder {
        Client::builder(endpoint)
            .nick(name)
            .realname("papo test peer")
            .allow_insecure_auth(true)
    }
}

fn unique_name(label: &str) -> String {
    let label: String = label
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .collect();
    unique(&label)
}

#[cfg(test)]
mod tests {
    use super::unique_name;

    #[test]
    fn names_are_readable_unique_and_bounded() {
        let first = unique_name("alice");
        let second = unique_name("alice");
        let process =
            std::env::var("PAPO_TEST_PID").unwrap_or_else(|_| std::process::id().to_string());
        assert!(first.starts_with("alice-"));
        assert!(first.contains(&format!("-{process}-")));
        assert_ne!(first, second);
        assert!(unique_name("a very long display label with spaces").len() <= 32);
    }
}
