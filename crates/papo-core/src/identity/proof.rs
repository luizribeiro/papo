use papo_proto::Nick;

use crate::{Account, RecipientGuard, RecipientStatus, ReplyToken, Session, State};

/// Proof that a person target was validated against session state.
///
/// This type has no public constructor. A proof can only be minted by the validating methods on
/// [`Session`] and [`State`].
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proven {
    pub(super) kind: ProvenKind,
    pub(super) origin: papo_proto::Origin,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ProvenKind {
    Person(RecipientGuard),
    CurrentName(Nick),
}

/// Why session state could not mint or resolve person-target proof.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProofFailure {
    /// The evidence belongs to another connection origin.
    ForeignReference,
    /// The supplied lines do not prove the requested user and account.
    MissingEvidence,
    /// The proven person has departed or the nickname changed hands.
    IdentityChanged,
}

impl State {
    /// Mints proof with explicit current-holder nickname semantics.
    #[doc(hidden)]
    #[must_use]
    pub fn __prove_current_name(&self, nick: Nick) -> Option<Proven> {
        (nick.origin() == self.__reply_token().origin).then(|| Proven {
            origin: nick.origin(),
            kind: ProvenKind::CurrentName(nick),
        })
    }

    fn prove_event_user(
        &self,
        token: ReplyToken,
        nick: Nick,
        account: Account,
    ) -> Result<RecipientGuard, ProofFailure> {
        if token.origin != self.__reply_token().origin || nick.origin() != token.origin {
            return Err(ProofFailure::ForeignReference);
        }
        if token.identity.is_none() {
            return Err(ProofFailure::MissingEvidence);
        }
        let recipient = RecipientGuard::from_parts(nick, account, token);
        match self.__resolve_recipient(&recipient) {
            RecipientStatus::Current(_) | RecipientStatus::Gone => Ok(recipient),
            RecipientStatus::Unknown => Err(ProofFailure::MissingEvidence),
        }
    }

    /// Validates line evidence and mints proof for an event user.
    #[doc(hidden)]
    pub fn __prove_event_user(
        &self,
        token: ReplyToken,
        nick: Nick,
        account: Account,
    ) -> Result<Proven, ProofFailure> {
        let recipient = self.prove_event_user(token, nick, account)?;
        Ok(Proven {
            origin: recipient.token.origin,
            kind: ProvenKind::Person(recipient),
        })
    }

    /// Converts a validated compatibility recipient into opaque person proof.
    #[doc(hidden)]
    pub fn __prove_recipient(&self, recipient: &RecipientGuard) -> Result<Proven, ProofFailure> {
        match self.__resolve_recipient(recipient) {
            RecipientStatus::Current(_) | RecipientStatus::Gone => Ok(Proven {
                kind: ProvenKind::Person(recipient.clone()),
                origin: self.__reply_token().origin,
            }),
            RecipientStatus::Unknown => Err(ProofFailure::MissingEvidence),
        }
    }

    /// Returns a validated compatibility guard for received line evidence.
    #[doc(hidden)]
    #[must_use]
    pub fn __recipient_from_event(
        &self,
        token: ReplyToken,
        nick: Nick,
        account: Account,
    ) -> Option<RecipientGuard> {
        self.prove_event_user(token, nick, account).ok()
    }

    /// Captures the currently tracked person under a nickname for compatibility callers.
    #[doc(hidden)]
    #[must_use]
    pub fn __capture_recipient(&self, nick: Nick, account: Account) -> Option<RecipientGuard> {
        let token = self.__reply_token_for(nick.as_str());
        self.prove_event_user(token, nick, account).ok()
    }

    /// Resolves validated proof to the nickname that may be written now.
    #[doc(hidden)]
    pub fn __resolve_proven(&self, proof: &Proven) -> Result<Nick, ProofFailure> {
        if proof.origin != self.__reply_token().origin {
            return Err(ProofFailure::ForeignReference);
        }
        match &proof.kind {
            ProvenKind::Person(recipient) => match self.__resolve_recipient(recipient) {
                RecipientStatus::Current(nick) => Ok(nick),
                RecipientStatus::Gone | RecipientStatus::Unknown => {
                    Err(ProofFailure::IdentityChanged)
                }
            },
            ProvenKind::CurrentName(nick) => Ok(nick.clone()),
        }
    }
}

impl Session {
    /// Validates line evidence and mints proof for an event user.
    #[doc(hidden)]
    #[must_use]
    pub fn __prove_event_user(
        &self,
        token: ReplyToken,
        nick: Nick,
        account: Account,
    ) -> Option<Proven> {
        self.state.__prove_event_user(token, nick, account).ok()
    }

    /// Mints proof with explicit current-holder nickname semantics.
    #[doc(hidden)]
    #[must_use]
    pub fn __prove_current_name(&self, nick: Nick) -> Option<Proven> {
        self.state.__prove_current_name(nick)
    }
}
