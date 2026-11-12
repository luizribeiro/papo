use tokio::sync::oneshot;

use crate::{Client, Error, Feature, driver::Command};

/// Handle for the declarative server-side monitor list.
pub struct Monitor {
    client: Client,
}

impl std::fmt::Debug for Monitor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("Monitor", formatter)
    }
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
        S: crate::AsNick,
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
        S: crate::AsNick,
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
        S: crate::AsNick,
    {
        let client = self.client.clone();
        let nicks = nicks
            .into_iter()
            .map(|nick| crate::AsNick::__parts(&nick).into_owned())
            .collect::<Vec<_>>();
        crate::RequestFuture::new(async move {
            let nicks = nicks
                .into_iter()
                .map(|nick| client.resolve(crate::target::Role::Nick("monitor: nick"), nick))
                .collect::<Result<Vec<_>, _>>()?;
            preflight(&client)?;
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
