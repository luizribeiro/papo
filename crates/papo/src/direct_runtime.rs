use crate::Error;

#[derive(Debug)]
pub(crate) struct Acquired {
    recipient: papo_core::RecipientGuard,
    confirmation: Option<tokio::sync::oneshot::Receiver<Result<(), Error>>>,
    lease: crate::direct::MonitorLease,
}

impl Acquired {
    pub(crate) fn new(
        recipient: papo_core::RecipientGuard,
        confirmation: Option<tokio::sync::oneshot::Receiver<Result<(), Error>>>,
        commands: tokio::sync::mpsc::Sender<crate::driver::Command>,
    ) -> Self {
        let lease = crate::direct::MonitorLease::new(recipient.clone(), commands);
        Self {
            recipient,
            confirmation,
            lease,
        }
    }

    pub(crate) async fn confirmed(mut self) -> Result<Self, Error> {
        if let Some(confirmation) = self.confirmation.take() {
            match confirmation.await.map_err(|_| Error::connection_lost())? {
                Ok(()) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(self)
    }

    pub(crate) fn into_parts(self) -> (papo_core::RecipientGuard, crate::direct::MonitorLease) {
        (self.recipient, self.lease)
    }
}

pub(crate) async fn acquire(client: &crate::Client, nick: String) -> Result<Acquired, Error> {
    let (result, receiver) = tokio::sync::oneshot::channel();
    client
        .commands
        .send(crate::driver::Command::DirectAcquire {
            nick,
            commands: client.commands.clone(),
            result,
        })
        .await
        .map_err(|_| Error::connection_lost())?;
    receiver.await.map_err(|_| Error::connection_lost())?
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use papo_core::{Config, Session};

    use super::*;

    fn client(session: &Session) -> crate::Client {
        let (commands, _) = tokio::sync::mpsc::channel(1);
        crate::Client {
            commands,
            state: Arc::new(RwLock::new(Arc::new(crate::State::from_session(
                session, false,
            )))),
            origin: crate::Origin::NONE,
            network_builder: None,
        }
    }

    #[tokio::test]
    async fn dropping_an_acquisition_releases_the_exact_recipient() {
        let mut session = Session::new(Config::default());
        let (commands, mut receiver) = tokio::sync::mpsc::channel(1);
        let mut client = client(&session);
        client.commands = commands;
        let recipient = session.__direct_acquire("alice", None, None, crate::Account::LoggedOut);

        drop(Acquired::new(
            recipient.clone(),
            None,
            client.commands.clone(),
        ));
        assert!(matches!(
            receiver.try_recv(),
            Ok(crate::driver::Command::DirectRelease { recipient: released })
                if released == recipient
        ));
    }
}
