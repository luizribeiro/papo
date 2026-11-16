use papo_proto::{AccountName, Line, Nick};

use crate::{Account, Session};

use super::proof::{ProofFailure, Proven, ProvenKind};

impl Session {
    /// Validates a matching WHOX nickname/account row and mints person proof.
    ///
    /// When `expected_account` is given, the row must show the user logged in to exactly that
    /// account: a logged-out row or another account is a different person holding the nick.
    #[doc(hidden)]
    pub fn __prove_who_user(
        &mut self,
        nick: Nick,
        expected_account: Option<&AccountName>,
        lines: &[Line],
    ) -> Result<Proven, ProofFailure> {
        if nick.origin() != self.state.__reply_token().origin {
            return Err(ProofFailure::ForeignReference);
        }
        let account = self
            .state
            .__who_account(nick.as_str(), lines)
            .ok_or(ProofFailure::MissingEvidence)?;
        if expected_account.is_some_and(|expected| account.name() != Some(expected)) {
            return Err(ProofFailure::IdentityChanged);
        }
        if let Some(user) = self.state.user(&nick)
            && user.account() != &Account::Unknown
            && user.account() != &account
        {
            return Err(ProofFailure::IdentityChanged);
        }
        let recipient = self.state.prove_who_identity(nick, account);
        self.state_generation = self.state_generation.wrapping_add(1);
        Ok(Proven {
            origin: recipient.token.origin,
            kind: ProvenKind::Person(recipient),
        })
    }
}
