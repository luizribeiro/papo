use papo_proto::{Line, Nick};

use crate::{Account, Session};

use super::proof::{ProofFailure, Proven, ProvenKind};

impl Session {
    /// Validates a matching WHOX nickname/account row and mints person proof.
    #[doc(hidden)]
    pub fn __prove_who_user(
        &mut self,
        nick: Nick,
        account: Account,
        lines: &[Line],
    ) -> Result<Proven, ProofFailure> {
        if nick.origin() != self.state.__reply_token().origin {
            return Err(ProofFailure::ForeignReference);
        }
        let row_matches = lines.iter().any(|line| {
            let line = line.as_ref();
            if !line.command_is("354") || line.param(1) != Some(nick.as_str()) {
                return false;
            }
            match (&account, line.param(2)) {
                (Account::LoggedIn(expected), Some(actual)) => expected == actual,
                (Account::LoggedOut, Some("0" | "*")) => true,
                (Account::Unknown | Account::LoggedIn(_) | Account::LoggedOut, _) => false,
            }
        });
        if !row_matches {
            return Err(ProofFailure::MissingEvidence);
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
