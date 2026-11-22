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
    if handle.is_private_history() {
        return Err(crate::Error::TargetGone);
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
            papo_proto::Line::new("WHO").with_params([nick.to_string(), "%na".into()]),
            Some(papo_core::ResponseMatcher::Who(nick.to_string())),
        )
        .await?;
    who_account(client, nick.as_str(), &response)?;
    let expected_account = handle.account().name().cloned();
    if handle.is_unproven_history() && expected_account.is_none() {
        return Err(crate::Error::TargetGone);
    }
    let (result, receiver) = tokio::sync::oneshot::channel();
    client
        .commands
        .send(crate::driver::Command::ProveUser {
            nick: nick.clone(),
            expected_account,
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
    prove_current_name(&state, nick)
}

pub(crate) fn prove_current_name(
    state: &crate::State,
    nick: crate::Nick,
) -> Result<ProvenUser, crate::Error> {
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
    crate::requests::empty(response)?;
    let state = client.state();
    let account = state
        .core
        .__who_account(nick, &response.lines)
        .ok_or(crate::Error::TargetGone)?;
    if matches!(account, crate::Account::LoggedIn(_)) {
        state
            .known_name_rules()
            .ok_or(crate::InvalidInput::NameRulesUnknown)?;
    }
    Ok(account)
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
}
