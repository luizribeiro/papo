pub(crate) struct ProvenUser {
    pub(crate) core: papo_core::Proven,
    pub(crate) nick: crate::Nick,
}

pub(crate) async fn prove(
    client: &crate::Client,
    handle: &crate::UserHandle,
) -> Result<ProvenUser, crate::Error> {
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

pub(crate) fn map_failure(failure: papo_core::ProofFailure) -> crate::Error {
    match failure {
        papo_core::ProofFailure::ForeignReference => crate::InvalidInput::ForeignReference.into(),
        papo_core::ProofFailure::MissingEvidence | papo_core::ProofFailure::IdentityChanged => {
            crate::Error::TargetGone
        }
        _ => crate::Error::TargetGone,
    }
}
