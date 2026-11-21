use std::{
    fmt,
    hash::{Hash, Hasher},
};

mod encoding;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PeerScope {
    pub(super) server: String,
    pub(super) bouncer: Option<BouncerScope>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct BouncerScope {
    pub(super) login: String,
    pub(super) network: String,
    pub(super) upstream: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum KeyScope {
    Network(PeerScope),
    Connection(String),
}

#[derive(Clone, Debug)]
pub(super) struct PeerThread {
    scope: KeyScope,
    casemapping: crate::Casemapping,
    nick: String,
}

#[derive(Clone, Eq, PartialEq)]
pub(super) struct AccountProof {
    casemapping: crate::Casemapping,
    account: String,
    id: Option<String>,
}

#[derive(Clone, Eq, PartialEq)]
enum PeerProof {
    Account(Box<AccountProof>, Option<papo_core::PeerToken>),
    Unverified(Option<papo_core::PeerToken>),
}

/// A saved key for a nickname-addressed direct conversation.
///
/// An Account proof is the security boundary as far as its TLS-authenticated server is trusted. It
/// identifies a server-asserted account after `005` established the casemapping, not a person:
/// account names can be transferred, compromised, dropped, and re-registered. Through a bouncer,
/// the authenticated bouncer host, bouncer login, network ID, and TLS upstream host all form the
/// scope. Account proofs can survive reconnects and process restarts.
///
/// Continuity proof is best-effort evidence that papo has observed one IRC user without a break on
/// this connection. It depends on papo seeing departures and must be presented as unverified. The
/// proof cannot be persisted, changes after lost continuity, and is unavailable when papo can
/// prove nothing. Its display form retains the thread with explicit unverified proof. A bouncer
/// upstream disconnect ends continuity for that network even though the downstream connection
/// remains open; a duplicate self-JOIN resync is treated the same way. Detecting these transitions
/// relies on soju's `BOUNCER_NETID`, `BouncerServ` notice, and `BOUNCER NETWORK` line, none of which
/// is part of any `IRCv3` specification. This can require applications to confirm a peer again
/// after a harmless upstream reconnect.
///
/// Native and WASI `ircs://` and `wss://` connections, and browser `wss://` connections, can
/// produce durable Account proofs. Plaintext, browser `ws://`, and caller-supplied streams produce
/// only Continuity proofs. The authenticated scope is the normalized hostname, without its port:
/// changing hostnames makes an old key [`PeerKeyMismatch::DifferentAccount`], or
/// [`PeerKeyMismatch::CannotVerify`] when the new connection has only Continuity. Applications
/// should migrate stored keys to a new hostname only as an explicit trust decision.
///
/// Equality and hashing use only the scope and folded [`crate::Nick::key`]. They deliberately
/// ignore proof, so upgrading proof does not move an application thread. Use [`Self::check`]
/// before trusting the current holder, and treat an [`InvalidPeerKey`] while loading storage as an
/// identity that cannot be verified.
///
/// Proof fields are private and cannot be constructed by applications:
///
/// ```compile_fail
/// let key = papo::PeerKey { proof: () };
/// ```
#[derive(Clone)]
pub struct PeerKey {
    thread: PeerThread,
    proof: PeerProof,
    encoded: String,
}

#[derive(Clone, Debug)]
/// Result of checking a stored peer key against the current key.
pub enum PeerKeyCheck {
    /// The keys identify the same account or observed user continuity.
    Match,
    /// The current key adds durable account proof or an immutable account ID.
    Upgrade(PeerKey),
    /// The keys cannot be accepted without user confirmation.
    Mismatch(PeerKeyMismatch),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
/// Exhaustive reason that a stored peer key does not match the current key.
pub enum PeerKeyMismatch {
    /// Both keys are durable but identify different accounts or authenticated scopes.
    DifferentAccount,
    /// At least one side lacks enough durable proof to verify the identity.
    CannotVerify,
}

/// An error returned when a persisted peer key is malformed or unsupported.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InvalidPeerKey;

impl PeerKey {
    fn new(thread: PeerThread, proof: PeerProof) -> Self {
        let encoded = encoding::render(&thread, &proof);
        Self {
            thread,
            proof,
            encoded,
        }
    }

    /// Checks this stored key against the current key for the same nickname thread.
    #[must_use]
    pub fn check(&self, current_key: &Self) -> PeerKeyCheck {
        if self.thread != current_key.thread {
            return if self.is_verified() && current_key.is_verified() {
                PeerKeyCheck::Mismatch(PeerKeyMismatch::DifferentAccount)
            } else {
                PeerKeyCheck::Mismatch(PeerKeyMismatch::CannotVerify)
            };
        }
        match (&self.proof, &current_key.proof) {
            (PeerProof::Unverified(Some(stored)), PeerProof::Unverified(Some(current)))
                if stored == current =>
            {
                PeerKeyCheck::Match
            }
            (PeerProof::Account(stored, _), PeerProof::Account(current, _)) => {
                check_accounts(stored, current, current_key)
            }
            (PeerProof::Unverified(Some(stored)), PeerProof::Account(_, Some(current)))
                if stored == current =>
            {
                PeerKeyCheck::Upgrade(current_key.clone())
            }
            _ => PeerKeyCheck::Mismatch(PeerKeyMismatch::CannotVerify),
        }
    }

    /// Reports whether this key has durable account proof at a TLS-authenticated scope.
    #[must_use]
    pub const fn is_verified(&self) -> bool {
        matches!(self.proof, PeerProof::Account(_, _))
    }

    /// Returns the stable representation when this key has durable account proof.
    #[must_use]
    pub fn as_verified_str(&self) -> Option<&str> {
        self.is_verified().then_some(&self.encoded)
    }
}

impl PartialEq for PeerThread {
    fn eq(&self, other: &Self) -> bool {
        self.scope == other.scope && self.nick == other.nick
    }
}

impl Eq for PeerThread {}

impl Hash for PeerThread {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (&self.scope, &self.nick).hash(state);
    }
}

fn check_accounts(
    stored: &AccountProof,
    current: &AccountProof,
    current_key: &PeerKey,
) -> PeerKeyCheck {
    match (&stored.id, &current.id) {
        (Some(stored), Some(current)) if stored == current => PeerKeyCheck::Match,
        (None, Some(_))
            if stored.casemapping == current.casemapping && stored.account == current.account =>
        {
            PeerKeyCheck::Upgrade(current_key.clone())
        }
        (Some(_), None) => PeerKeyCheck::Mismatch(PeerKeyMismatch::CannotVerify),
        (None, None)
            if stored.casemapping == current.casemapping && stored.account == current.account =>
        {
            PeerKeyCheck::Match
        }
        (Some(_), Some(_)) | (None, Some(_) | None) => {
            PeerKeyCheck::Mismatch(PeerKeyMismatch::DifferentAccount)
        }
    }
}

impl PartialEq for PeerKey {
    fn eq(&self, other: &Self) -> bool {
        self.thread == other.thread
    }
}

impl Eq for PeerKey {}

impl Hash for PeerKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.thread.hash(state);
    }
}

impl fmt::Debug for PeerKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PeerKey")
            .field("verified", &self.is_verified())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for PeerKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.encoded)
    }
}

impl fmt::Display for InvalidPeerKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid persisted peer key")
    }
}

impl std::error::Error for InvalidPeerKey {}
