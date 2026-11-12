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
    let account = crate::direct_runtime::who_account(client, nick.as_str(), &response)?;
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

pub(crate) fn map_failure(failure: papo_core::ProofFailure) -> crate::Error {
    match failure {
        papo_core::ProofFailure::ForeignReference => crate::ForeignReference.into(),
        _ => crate::Error::TargetGone,
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use papo_core::ProofFailure;
    use tokio::io::AsyncWriteExt as _;

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
}
