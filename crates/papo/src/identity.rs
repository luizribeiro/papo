pub(crate) struct ProvenUser {
    pub(crate) core: papo_core::Proven,
    pub(crate) nick: crate::Nick,
}

pub(crate) fn prove(
    client: &crate::Client,
    handle: &crate::UserHandle,
) -> Result<ProvenUser, crate::Error> {
    if handle.nick().origin() != client.origin() {
        return Err(crate::ForeignReference.into());
    }
    if handle.is_private_history() {
        return Err(crate::Error::TargetGone);
    }
    let proof = handle.proof().ok_or(crate::Error::TargetGone)?;
    let nick = client
        .state()
        .core
        .__resolve_proven(proof)
        .map_err(map_failure)?;
    Ok(ProvenUser {
        core: proof.clone(),
        nick,
    })
}

pub(crate) fn prove_parts(
    client: &crate::Client,
    parts: crate::target::sealed::Parts<'static>,
    argument: &'static str,
) -> Result<ProvenUser, crate::Error> {
    if let Some(handle) = parts.user.as_ref() {
        return prove(client, handle);
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

pub(crate) fn map_failure(failure: papo_core::ProofFailure) -> crate::Error {
    match failure {
        papo_core::ProofFailure::ForeignReference => crate::ForeignReference.into(),
        _ => crate::Error::TargetGone,
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use papo_core::{Config, ProofFailure, Session};

    use crate::{Account, Error};

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

    #[tokio::test]
    async fn past_line_handle_is_gone_without_a_lookup() {
        let mut session = Session::new(Config::default().nick("papo"));
        let now = crate::rt::now_mono();
        for line in [
            ":ergo.test 005 papo CASEMAPPING=rfc1459 :are supported",
            ":alice!~alice@cloak.example PRIVMSG papo :release is ready",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);
        let state = client.state();
        let names = state.name_rules().unwrap();
        let actor = crate::UserHandle::from_past_line(
            names.nick("alice"),
            Account::LoggedIn(names.account("alice-account")),
        );

        assert!(matches!(
            client.privmsg(actor, "thanks").await,
            Err(Error::TargetGone)
        ));
        assert!(commands.try_recv().is_err());
    }
}
