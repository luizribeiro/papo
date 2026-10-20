use tokio::sync::oneshot;

use crate::{Client, Error, Feature, InvalidInput, driver::Command};

/// Handle for the declarative server-side monitor list.
pub struct Monitor {
    client: Client,
}

impl Client {
    /// Returns a handle for changing the desired monitor list.
    #[must_use]
    pub fn monitor(&self) -> Monitor {
        Monitor {
            client: self.clone(),
        }
    }
}

impl Monitor {
    /// Adds nicknames to the desired monitor list.
    ///
    /// The desired list survives reconnects.
    ///
    /// # Errors
    ///
    /// Returns an unsupported, invalid-input, or disconnected error without sending when the
    /// request cannot be applied.
    pub fn add<I, S>(&self, nicks: I) -> crate::RequestFuture<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.change(nicks, true)
    }

    /// Removes nicknames from the desired monitor list.
    ///
    /// # Errors
    ///
    /// Returns an unsupported, invalid-input, or disconnected error without sending when the
    /// request cannot be applied.
    pub fn remove<I, S>(&self, nicks: I) -> crate::RequestFuture<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.change(nicks, false)
    }

    /// Clears the desired monitor list.
    ///
    /// # Errors
    ///
    /// Returns an unsupported or disconnected error without sending when the request cannot be
    /// applied.
    pub fn clear(&self) -> crate::RequestFuture<()> {
        let client = self.client.clone();
        crate::RequestFuture::new(async move {
            preflight(&client)?;
            let (result, receiver) = oneshot::channel();
            client
                .commands
                .send(Command::MonitorClear { result })
                .await
                .map_err(|_| Error::connection_lost())?;
            receiver.await.map_err(|_| Error::connection_lost())?
        })
    }

    fn change<I, S>(&self, nicks: I, add: bool) -> crate::RequestFuture<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let client = self.client.clone();
        let nicks = nicks
            .into_iter()
            .map(|nick| nick.as_ref().to_owned())
            .collect::<Vec<_>>();
        crate::RequestFuture::new(async move {
            preflight(&client)?;
            let state = client.state();
            for nick in &nicks {
                state
                    .nickname(nick)
                    .check(state.isupport().inner())
                    .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            }
            let (result, receiver) = oneshot::channel();
            let command = if add {
                Command::MonitorAdd { nicks, result }
            } else {
                Command::MonitorRemove { nicks, result }
            };
            client
                .commands
                .send(command)
                .await
                .map_err(|_| Error::connection_lost())?;
            receiver.await.map_err(|_| Error::connection_lost())?
        })
    }
}

fn preflight(client: &Client) -> Result<(), Error> {
    let state = client.state();
    if !state.is_connected() {
        return Err(Error::connection_lost());
    }
    Client::require_feature(&state, Feature::Monitor)
}

#[cfg(all(test, not(browser)))]
mod tests {
    use crate::driver::test_support::*;
    use crate::{Error, Feature};
    use tokio::io::AsyncReadExt;

    #[tokio::test(flavor = "current_thread")]
    async fn monitor_mutations_reach_the_wire() {
        let (mut server, client, mut events, driver) = scripted(config());
        let driver = tokio::spawn(driver);
        welcome_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(&mut server, b":ergo.test 001 papo-driver :Welcome\r\n:ergo.test 005 papo-driver MONITOR=100 NICKLEN=32 :are supported\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Connected(_))));

        let add = tokio::spawn({
            let client = client.clone();
            async move { client.monitor().add(["alice", "bob"]).await }
        });
        client_line(&mut server, b"MONITOR + alice,bob\r\n").await;
        assert!(within(add).await.unwrap().is_ok());
        assert_eq!(
            client
                .state()
                .monitored()
                .map(crate::Nick::as_ref)
                .collect::<Vec<_>>(),
            ["alice", "bob"]
        );

        let remove = tokio::spawn({
            let client = client.clone();
            async move {
                let nick = client.state().casemapping().nick("bob");
                client.monitor().remove([nick]).await
            }
        });
        client_line(&mut server, b"MONITOR - bob\r\n").await;
        assert!(within(remove).await.unwrap().is_ok());
        assert_eq!(
            client
                .state()
                .monitored()
                .map(crate::Nick::as_ref)
                .collect::<Vec<_>>(),
            ["alice"]
        );

        let clear = tokio::spawn(async move { client.monitor().clear().await });
        client_line(&mut server, b"MONITOR C\r\n").await;
        assert!(within(clear).await.unwrap().is_ok());
        abort_task(driver).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn monitor_rejects_missing_support_before_sending() {
        let (mut server, client, mut events, driver) = scripted(config());
        let driver = tokio::spawn(driver);
        complete_connect(&mut server, &mut events).await;

        assert!(matches!(
            client
                .monitor()
                .add([client.state().casemapping().nick("alice")])
                .await,
            Err(Error::Unsupported(error)) if error.feature == Feature::Monitor
        ));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1), server.read_u8())
                .await
                .is_err()
        );
        abort_task(driver).await;
    }
}
