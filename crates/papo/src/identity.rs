pub(crate) struct ProvenUser {
    pub(crate) core: papo_core::Proven,
    pub(crate) nick: crate::Nick,
}

pub(crate) async fn prove(
    client: &crate::Client,
    handle: &crate::UserHandle,
) -> Result<ProvenUser, crate::Error> {
    if handle.nick().origin() != client.origin() {
        return Err(crate::ForeignReference.into());
    }
    if let Some(proof) = handle.proof() {
        let nick = client
            .state()
            .core
            .__resolve_proven(proof)
            .map_err(map_failure)?;
        return Ok(ProvenUser {
            core: proof.clone(),
            nick,
        });
    }
    let nick = handle.nick();
    let response = client
        .correlated(
            crate::Line::new("WHO").with_params([nick.to_string(), "%na".into()]),
            Some(papo_core::ResponseMatcher::Who(nick.to_string())),
        )
        .await?;
    let account = who_account(client, nick.as_str(), &response)?;
    if handle.is_unproven_history() && !matches!(handle.account(), crate::Account::LoggedIn(_)) {
        return Err(crate::Error::TargetGone);
    }
    if let (crate::Account::LoggedIn(expected), crate::Account::LoggedIn(actual)) =
        (handle.account(), &account)
        && expected != actual
    {
        return Err(crate::Error::TargetGone);
    }
    let (result, receiver) = tokio::sync::oneshot::channel();
    client
        .commands
        .send(crate::driver::Command::ProveUser {
            nick: nick.clone(),
            account,
            lines: response.lines,
            result,
        })
        .await
        .map_err(|_| crate::Error::connection_lost())?;
    let proof = receiver
        .await
        .map_err(|_| crate::Error::connection_lost())??;
    handle.remember(proof.clone());
    Ok(ProvenUser { core: proof, nick })
}

pub(crate) async fn prove_parts(
    client: &crate::Client,
    parts: crate::target::sealed::Parts<'static>,
    argument: &'static str,
) -> Result<ProvenUser, crate::Error> {
    if let Some(handle) = parts.user.as_ref() {
        return prove(client, handle).await;
    }
    let nick = client.resolve(crate::target::Role::Nick(argument), parts)?;
    let state = client.state();
    let nick = state
        .known_name_rules()
        .ok_or(crate::InvalidInput::NameRulesUnknown)?
        .nick(nick);
    let core = state
        .core
        .__prove_current_name(nick.clone())
        .ok_or(crate::ForeignReference)?;
    Ok(ProvenUser { core, nick })
}

pub(crate) fn who_account(
    client: &crate::Client,
    nick: &str,
    response: &papo_core::Response,
) -> Result<crate::Account, crate::Error> {
    if let Some(error) = response.error() {
        return Err(crate::Error::server(error));
    }
    let account = response.lines.iter().find_map(|line| {
        let line = line.as_ref();
        (line.command_is("354") && line.param(1).is_some_and(|found| found == nick))
            .then(|| line.param(2))
            .flatten()
    });
    match account {
        None => Err(crate::Error::TargetGone),
        Some("0" | "*") => Ok(crate::Account::LoggedOut),
        Some(account) => Ok(crate::Account::LoggedIn(
            client
                .state()
                .known_name_rules()
                .ok_or(crate::InvalidInput::NameRulesUnknown)?
                .account(account),
        )),
    }
}

pub(crate) fn map_failure(failure: papo_core::ProofFailure) -> crate::Error {
    match failure {
        papo_core::ProofFailure::ForeignReference => crate::ForeignReference.into(),
        _ => crate::Error::TargetGone,
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::sync::{Arc, RwLock};

    use papo_core::{Config, Event, ProofFailure, ResponseMatcher, Session};
    use papo_proto::{Casemapping, Line, LineRef};
    use tokio::io::AsyncWriteExt as _;

    use super::who_account;
    use crate::{Account, Error};

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
                Event::Response(response) => Some(response),
                _ => None,
            })
            .unwrap();
        assert_eq!(response.id, id);
        response.lines = lines
            .iter()
            .map(|line| LineRef::parse(line).unwrap().to_owned())
            .collect();
        response
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

    #[test]
    fn foreign_proof_failure_stays_typed() {
        assert!(matches!(
            super::map_failure(ProofFailure::ForeignReference),
            crate::Error::ForeignReference(crate::ForeignReference)
        ));
    }

    #[test]
    fn missing_proof_evidence_means_the_target_is_gone() {
        assert!(matches!(
            super::map_failure(ProofFailure::MissingEvidence),
            crate::Error::TargetGone
        ));
    }

    #[test]
    fn changed_proof_identity_means_the_target_is_gone() {
        assert!(matches!(
            super::map_failure(ProofFailure::IdentityChanged),
            crate::Error::TargetGone
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn accountless_history_actor_cannot_be_proved_for_a_reply() {
        use crate::driver::test_support::{
            client_line, complete_connect, config, scripted, server_line, within,
        };

        let (mut server, client, mut events, driver) = scripted(config());
        let driver = tokio::spawn(driver);
        complete_connect(&mut server, &mut events).await;
        let nick = client.state().name_rules().unwrap().nick("alice");
        let handle = crate::UserHandle::from_event(
            nick,
            crate::Account::Unknown,
            None,
            Some(&client.state),
            true,
        );

        let proving = tokio::spawn({
            let client = client.clone();
            async move { super::prove(&client, &handle).await }
        });
        client_line(&mut server, b"WHO alice %na\r\n").await;
        server_line(
            &mut server,
            b":ergo.test 354 papo-driver alice current-account\r\n:ergo.test 315 papo-driver alice :End of WHO list\r\n",
        )
        .await;
        assert!(matches!(
            within(proving).await.unwrap(),
            Err(crate::Error::TargetGone)
        ));

        within(server.shutdown()).await.unwrap();
        assert!(within(driver).await.unwrap().is_err());
    }

    async fn next_command(
        commands: &mut tokio::sync::mpsc::Receiver<crate::driver::Command>,
    ) -> crate::driver::Command {
        tokio::time::timeout(std::time::Duration::from_millis(50), commands.recv())
            .await
            .expect("driver did not receive a command")
            .expect("driver command channel closed")
    }

    fn unproven_alice(session: &Session) -> (crate::UserHandle, Account) {
        let mapping = session.isupport().casemapping();
        let account = Account::LoggedIn(mapping.account("alice-account"));
        let handle =
            crate::UserHandle::from_parts(mapping.nick("alice"), account.clone(), None, None);
        (handle, account)
    }

    #[tokio::test]
    async fn unproven_recipient_send_requires_the_same_verified_account() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            now,
            b":ergo.test 005 papo CASEMAPPING=rfc1459 MONITOR=100 :are supported",
        );
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);
        let (alice, account) = unproven_alice(&session);
        let resolving = tokio::spawn({
            let client = client.clone();
            async move { client.privmsg(alice, "release approved").await }
        });

        let crate::driver::Command::Request {
            message, result, ..
        } = next_command(&mut commands).await
        else {
            panic!("expected WHO request")
        };
        assert_eq!(message.params(), ["alice", "%na"]);
        let response = response_with(&[":ergo.test 354 papo alice alice-account"]);
        result.send(Ok(response)).unwrap();
        let crate::driver::Command::ProveUser {
            nick,
            account: actual,
            lines,
            result,
        } = next_command(&mut commands).await
        else {
            panic!("expected user proof")
        };
        assert_eq!(actual, account);
        let proof = session.__prove_who_user(nick, actual, &lines).unwrap();
        result.send(Ok(proof)).unwrap();
        assert!(matches!(
            next_command(&mut commands).await,
            crate::driver::Command::SendPerson { .. }
        ));
        resolving.abort();
    }

    #[tokio::test]
    async fn unproven_current_account_requires_who_proof() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        for line in [
            b":ergo.test 005 papo CASEMAPPING=rfc1459 MONITOR=100 :are supported".as_slice(),
            b":papo!~papo@localhost JOIN #review",
            b":alice!~alice@cloak.example JOIN #review",
        ] {
            session.handle_line(now, line);
        }
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);
        let (alice, _) = unproven_alice(&session);
        let resolving =
            tokio::spawn(async move { client.privmsg(alice, "release approved").await });

        let crate::driver::Command::Request { message, .. } = next_command(&mut commands).await
        else {
            panic!("expected WHO request")
        };
        assert_eq!(message.params(), ["alice", "%na"]);
        resolving.abort();
    }
}
