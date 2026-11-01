use std::{fmt, str::FromStr};

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

impl PeerScope {
    pub(crate) fn server(server: &str) -> Self {
        Self {
            server: server.to_owned(),
            bouncer: None,
        }
    }
}

/// An identity pin for a nickname-addressed direct conversation.
///
/// An Account pin is the security boundary as far as its TLS-authenticated server is trusted. It
/// identifies a server-asserted account after `005` established the casemapping, not a person:
/// account names can be transferred, compromised, dropped, and re-registered. Through a bouncer,
/// the authenticated bouncer host, bouncer login, network ID, and TLS upstream host all form the
/// scope. Account pins are persistable and can survive reconnects and process restarts.
///
/// A Continuity pin is best-effort evidence that papo has observed one IRC user without a break on
/// this connection. It depends on papo seeing departures and must be presented as unverified. It
/// cannot be persisted, changes after lost continuity, and is unavailable when papo can prove
/// nothing. A bouncer upstream disconnect ends continuity for that network even though the
/// downstream connection remains open; a duplicate self-JOIN resync is treated the same way.
/// This can require applications to confirm a peer again after a harmless upstream reconnect.
///
/// Native and WASI `ircs://` and `wss://` connections, and browser `wss://` connections, can
/// produce durable Account pins. Plaintext, browser `ws://`, and caller-supplied streams produce
/// only Continuity pins. The authenticated scope is the normalized hostname, without its port:
/// changing hostnames makes an old Account pin [`PeerKeyMismatch::DifferentAccount`], or
/// [`PeerKeyMismatch::CannotVerify`] when the new connection has only Continuity. Applications
/// should migrate stored pins to a new hostname only as an explicit trust decision.
///
/// Pins do not key conversations. Store one beside the folded [`crate::Nick::key`] for each thread;
/// two nicknames logged into one account remain different threads. Use [`Self::check`], never raw
/// equality, and treat an [`InvalidPeerKey`] while loading storage as identity that cannot be
/// verified.
///
/// ```no_run
/// fn inspect_incoming(stored: &papo::PeerKey, sender: &papo::UserSource) {
///     let Some(current) = sender.peer_key() else { return };
///     match stored.check(current) {
///         papo::PeerKeyCheck::Match => {}
///         papo::PeerKeyCheck::Upgrade(pin) => persist(pin),
///         papo::PeerKeyCheck::Mismatch(reason) => warn(reason, current),
///     }
/// }
/// # fn persist(_: papo::PeerKey) {}
/// # fn warn(_: papo::PeerKeyMismatch, _: &papo::PeerKey) {}
///
/// async fn open(
///     client: &papo::Client,
///     nick: &papo::Nick,
///     stored: &papo::PeerKey,
/// ) -> Result<papo::DirectRef, papo::Error> {
///     match client.direct_pinned(nick, stored).await {
///         Ok(peer) => Ok(peer),
///         Err(papo::Error::PeerPinMismatch(mismatch)) => {
///             warn(mismatch.reason(), mismatch.current_key());
///             client.direct(nick).await // only after the user approves replacement
///         }
///         Err(error) => Err(error),
///     }
/// }
/// ```
#[derive(Clone)]
pub struct PeerKey(PeerIdentity);

#[derive(Clone, Eq, Hash, PartialEq)]
enum PeerIdentity {
    Account(AccountPin),
    Continuity(papo_core::PeerToken),
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct AccountPin {
    scope: PeerScope,
    casemapping: crate::Casemapping,
    account: String,
    id: Option<String>,
    encoded: String,
}

#[derive(Clone, Debug)]
/// Result of checking a stored peer pin against the current pin.
pub enum PeerKeyCheck {
    /// The pins identify the same account or observed user continuity.
    Match,
    /// The account name still matches and the current pin adds an immutable account ID.
    Upgrade(PeerKey),
    /// The pins cannot be accepted without user confirmation.
    Mismatch(PeerKeyMismatch),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
/// Exhaustive reason that a stored peer pin does not match the current pin.
pub enum PeerKeyMismatch {
    /// Both pins are durable but identify different accounts or authenticated scopes.
    DifferentAccount,
    /// At least one side lacks enough durable proof to verify the identity.
    CannotVerify,
}

/// A stored peer pin that did not authorize opening the current identity.
#[derive(Clone, Debug)]
pub struct PeerPinMismatch {
    reason: PeerKeyMismatch,
    current: Box<PeerKey>,
}

impl PeerPinMismatch {
    pub(crate) fn new(reason: PeerKeyMismatch, current: PeerKey) -> Self {
        Self {
            reason,
            current: Box::new(current),
        }
    }

    /// Returns why the current identity could not be accepted.
    #[must_use]
    pub const fn reason(&self) -> PeerKeyMismatch {
        self.reason
    }

    /// Returns the current pin for an explicit user-approved replacement.
    #[must_use]
    pub const fn current_key(&self) -> &PeerKey {
        &self.current
    }
}

/// An error returned when a persisted peer key is malformed or unsupported.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InvalidPeerKey;

impl PeerKey {
    pub(crate) fn live(
        scope: Option<&PeerScope>,
        account: &crate::Account,
        token: papo_core::PeerToken,
    ) -> Option<Self> {
        Self::new(scope, account, token.__is_known().then_some(token))
    }

    pub(crate) fn historical(scope: Option<&PeerScope>, account: &crate::Account) -> Option<Self> {
        match (scope, account) {
            (Some(scope), crate::Account::LoggedIn(account)) => {
                Some(Self::verified(scope, account.casemapping(), account.key()))
            }
            _ => None,
        }
    }

    fn new(
        scope: Option<&PeerScope>,
        account: &crate::Account,
        tracked: Option<papo_core::PeerToken>,
    ) -> Option<Self> {
        if let (Some(scope), crate::Account::LoggedIn(account)) = (scope, account) {
            return Some(Self::verified(scope, account.casemapping(), account.key()));
        }
        tracked.map(|token| Self(PeerIdentity::Continuity(token)))
    }

    fn verified(scope: &PeerScope, casemapping: crate::Casemapping, account: &str) -> Self {
        Self(PeerIdentity::Account(encoding::encode(
            scope,
            casemapping,
            account,
        )))
    }

    /// Checks this stored pin against the current pin for the same nickname thread.
    ///
    /// A stored Account pin without an immutable account ID returns [`PeerKeyCheck::Upgrade`] once
    /// the server supplies that ID. Applications must replace the stored pin with the upgraded pin.
    #[must_use]
    pub fn check(&self, current: &Self) -> PeerKeyCheck {
        match (&self.0, &current.0) {
            (PeerIdentity::Continuity(stored), PeerIdentity::Continuity(current)) => {
                if stored == current {
                    PeerKeyCheck::Match
                } else {
                    PeerKeyCheck::Mismatch(PeerKeyMismatch::CannotVerify)
                }
            }
            (PeerIdentity::Account(stored), PeerIdentity::Account(current)) => {
                check_accounts(stored, current)
            }
            (PeerIdentity::Account(_), PeerIdentity::Continuity(_))
            | (PeerIdentity::Continuity(_), PeerIdentity::Account(_)) => {
                PeerKeyCheck::Mismatch(PeerKeyMismatch::CannotVerify)
            }
        }
    }

    /// Reports whether this is a durable Account pin at a TLS-authenticated server scope.
    #[must_use]
    pub const fn is_verified(&self) -> bool {
        matches!(self.0, PeerIdentity::Account(_))
    }

    /// Returns the stable representation of a verified pin.
    ///
    /// Continuity pins return `None` so they cannot accidentally be stored as durable identities.
    /// A returned value round-trips through [`str::parse`](FromStr::from_str).
    #[must_use]
    pub fn as_verified_str(&self) -> Option<&str> {
        match &self.0 {
            PeerIdentity::Account(pin) => Some(&pin.encoded),
            PeerIdentity::Continuity(_) => None,
        }
    }
}

fn check_accounts(stored: &AccountPin, current: &AccountPin) -> PeerKeyCheck {
    if stored.scope != current.scope {
        return PeerKeyCheck::Mismatch(PeerKeyMismatch::DifferentAccount);
    }
    match (&stored.id, &current.id) {
        (Some(stored), Some(current)) if stored == current => PeerKeyCheck::Match,
        (None, Some(_))
            if stored.casemapping == current.casemapping && stored.account == current.account =>
        {
            PeerKeyCheck::Upgrade(PeerKey(PeerIdentity::Account(current.clone())))
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

impl FromStr for PeerKey {
    type Err = InvalidPeerKey;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        encoding::parse(value).map(|pin| Self(PeerIdentity::Account(pin)))
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

impl fmt::Display for InvalidPeerKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid persisted peer key")
    }
}

impl fmt::Display for PeerPinMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.reason {
            PeerKeyMismatch::DifferentAccount => formatter.write_str("the peer account changed"),
            PeerKeyMismatch::CannotVerify => {
                formatter.write_str("the current peer identity cannot be verified")
            }
        }
    }
}

impl std::error::Error for InvalidPeerKey {}

impl std::error::Error for PeerPinMismatch {}
