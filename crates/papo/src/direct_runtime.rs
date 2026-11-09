use crate::{Account, Error};

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

pub(crate) async fn refresh(
    client: &crate::Client,
    recipient: papo_core::RecipientGuard,
    account: Account,
) -> Result<papo_core::RecipientGuard, Error> {
    let (result, receiver) = tokio::sync::oneshot::channel();
    client
        .commands
        .send(crate::driver::Command::DirectProve {
            recipient,
            account,
            result,
        })
        .await
        .map_err(|_| Error::connection_lost())?;
    receiver.await.map_err(|_| Error::connection_lost())?
}

pub(crate) async fn prove(
    client: &crate::Client,
    nick: String,
    acquired: Acquired,
) -> Result<(Acquired, Account), Error> {
    let mut acquired = acquired;
    let recipient = acquired.recipient.clone();
    let confirmation = acquired.confirmation.take();
    let response = client.correlated(
        crate::Line::new("WHO").with_params([nick.clone(), "%na".to_owned()]),
        Some(papo_core::ResponseMatcher::Who(nick.clone())),
    );
    tokio::pin!(response);
    let response = if let Some(mut confirmation) = confirmation {
        tokio::select! {
            confirmed = &mut confirmation => match confirmed {
                Ok(Ok(())) => response.await,
                Ok(Err(error)) => Err(error),
                Err(_) => Err(Error::connection_lost()),
            },
            response = &mut response => match confirmation.try_recv() {
                Ok(Err(error)) => Err(error),
                _ => response,
            },
        }
    } else {
        response.await
    };
    let response = match response {
        Ok(response) => response,
        Err(error) => return Err(error),
    };
    let account = match who_account(client, &nick, &response) {
        Ok(account) => account,
        Err(error) => return Err(error),
    };
    match refresh(client, recipient, account.clone()).await {
        Ok(recipient) => {
            acquired.recipient = recipient;
            Ok((acquired, account))
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn require_monitor_slot(state: &crate::State, nick: &str) -> Result<(), Error> {
    crate::Client::require_feature(state, crate::Feature::Monitor)?;
    if state.monitored().any(|known| known == nick) {
        return Ok(());
    }
    if let Some(crate::isupport::LimitValue::Limited(limit)) = state.isupport().monitor()
        && state.monitored().len() >= limit
    {
        return Err(Error::MonitorFull { limit });
    }
    Ok(())
}

pub(crate) fn who_account(
    client: &crate::Client,
    nick: &str,
    response: &papo_core::Response,
) -> Result<Account, Error> {
    if let Some(error) = response.error() {
        return Err(Error::server(error));
    }
    let account = response.lines.iter().find_map(|line| {
        let line = line.as_ref();
        (line.command_is("354") && line.param(1).is_some_and(|found| found == nick))
            .then(|| line.param(2))
            .flatten()
    });
    match account {
        None => Err(Error::TargetGone),
        Some("0" | "*") => Ok(Account::LoggedOut),
        Some(account) => Ok(Account::LoggedIn(
            client
                .state()
                .known_name_rules()
                .ok_or(crate::InvalidInput::NameRulesUnknown)?
                .account(account),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use papo_core::{Config, Event, ResponseMatcher, Session};
    use papo_proto::{Casemapping, Line, LineRef};

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

    fn response_with(lines: &[&str]) -> papo_core::Response {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default());
        let id = session
            .request_with_matcher(
                now,
                Line::new("WHO").with_params(["alice", "%na"]),
                ResponseMatcher::Who("alice".into()),
            )
            .unwrap();
        _ = session.poll_transmit();
        session.handle_line(now, b":ergo.test 315 papo alice :End of WHO list");
        let mut response = std::iter::from_fn(|| session.poll_event())
            .find_map(|event| match event {
                Event::Response(response) if response.id == id => Some(response),
                _ => None,
            })
            .unwrap();
        response.lines = lines
            .iter()
            .map(|line| LineRef::parse(line).unwrap().to_owned())
            .collect();
        response
    }

    #[test]
    fn monitor_capacity_uses_the_advertised_literal_boundary() {
        let mut session = Session::new(Config::default());
        session.handle_line(
            crate::rt::now_mono(),
            b":ergo.test 001 papo :Welcome to ErgoTest",
        );
        session.handle_line(
            crate::rt::now_mono(),
            b":ergo.test 005 papo MONITOR=100 :are supported",
        );
        session.handle_line(crate::rt::now_mono(), b":ergo.test 376 papo :End of MOTD");
        session.monitor_add((0..99).map(|index| format!("reviewer{index}")));
        assert!(
            require_monitor_slot(&crate::State::from_session(&session, false), "alice").is_ok()
        );

        session.monitor_add(["reviewer99"]);
        let state = crate::State::from_session(&session, false);
        assert!(matches!(
            require_monitor_slot(&state, "alice"),
            Err(Error::MonitorFull { limit: 100 })
        ));
        assert!(require_monitor_slot(&state, "reviewer42").is_ok());
    }

    #[tokio::test]
    async fn dropping_an_acquisition_releases_the_exact_recipient() {
        let mut session = Session::new(Config::default());
        let (commands, mut receiver) = tokio::sync::mpsc::channel(1);
        let mut client = client(&session);
        client.commands = commands;
        let recipient = session.__direct_acquire("alice", None, None, Account::LoggedOut);

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

    #[test]
    fn who_account_requires_a_matching_who_account_row() {
        let session = Session::new(Config::default().casemapping(Casemapping::default()));
        let client = client(&session);
        for (line, expected) in [
            (
                ":ergo.test 354 papo alice alice-account",
                Some("alice-account"),
            ),
            (":ergo.test 354 papo alice 0", None),
            (":ergo.test 354 papo alice *", None),
        ] {
            let account = who_account(&client, "alice", &response_with(&[line])).unwrap();
            match expected {
                Some(expected) => {
                    assert!(matches!(account, Account::LoggedIn(name) if name == expected));
                }
                None => assert_eq!(account, Account::LoggedOut),
            }
        }
        for line in [
            ":ergo.test 352 papo alice mallory-account",
            ":ergo.test 354 papo mallory mallory-account",
        ] {
            assert!(matches!(
                who_account(&client, "alice", &response_with(&[line])),
                Err(Error::TargetGone)
            ));
        }
    }
}
