use std::{fmt, str::FromStr};

mod encoding;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PeerScope {
    server: String,
    bouncer: Option<BouncerScope>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct BouncerScope {
    login: String,
    network: String,
    upstream: String,
}

impl PeerScope {
    pub(crate) fn server(server: &str) -> Self {
        Self {
            server: server.to_owned(),
            bouncer: None,
        }
    }

    #[cfg(feature = "unstable")]
    pub(crate) fn bouncer(server: &str, login: &str, network: &str, upstream: &str) -> Self {
        Self {
            server: server.to_owned(),
            bouncer: Some(BouncerScope {
                login: login.to_owned(),
                network: network.to_owned(),
                upstream: upstream.to_owned(),
            }),
        }
    }
}

/// An identity pin for a nickname-addressed direct conversation.
///
/// Verified pins compare the account under the server's advertised casemapping together with the
/// server name authenticated by TLS. A bound bouncer network ID is part of that scope. They remain
/// equal across reconnects, process restarts, and multiple simultaneous nicknames logged into the
/// same account. Connecting through another host deliberately produces a different pin.
///
/// A `PeerKey` is not a conversation key. Applications key threads by the nickname's folded
/// [`crate::Nick::key`] within their configured connection, and store this value as the identity
/// pinned to that thread. Two nicknames on one account have equal pins but remain separate threads;
/// a nickname change starts a new thread even though the live user's pin remains equal.
///
/// Plaintext connections and caller-supplied streams produce only session pins because papo did
/// not verify a certificate for their server name. `ISUPPORT NETWORK` is descriptive and never
/// affects identity. Session pins remain equal only while papo can prove continuity of one IRC
/// user. They change after reconnect, departure and nickname reuse, or account logout.
///
/// Only [`Self::as_verified_str`] exposes a persistable representation. Parsing accepts that
/// representation, while session pins deliberately have no string form.
///
/// Store the folded nickname key and pin together. Compare the current pin before routing an
/// incoming message or sending, and replace the pin only after the user accepts a warning:
///
/// ```no_run
/// fn warning(stored: &papo::PeerKey, current: &papo::PeerKey) -> Option<&'static str> {
///     (stored != current).then(|| {
///         if stored.is_verified() && current.is_verified() {
///             "different account"
///         } else {
///             "cannot verify identity"
///         }
///     })
/// }
///
/// fn inspect_incoming(thread: (&str, &papo::PeerKey), sender: &papo::UserSource) {
///     if sender.nick().key() == thread.0
///         && let Some(current) = sender.peer_key()
///         && let Some(alert) = warning(thread.1, current)
///     {
///         eprintln!("{alert}");
///     }
/// }
///
/// async fn current_peer(
///     client: &papo::Client,
///     nick: &papo::Nick,
///     stored: &papo::PeerKey,
///     accept: &mut impl FnMut(&str) -> bool,
/// ) -> Result<Option<papo::DirectRef>, papo::Error> {
///     let peer = client.direct(nick).await?;
///     if let Some(alert) = warning(stored, peer.peer_key()) {
///         if !accept(alert) { return Ok(None); }
///     }
///     Ok(Some(peer))
/// }
///
/// async fn send(
///     client: &papo::Client,
///     nick: &papo::Nick,
///     stored: &papo::PeerKey,
///     mut accept: impl FnMut(&str) -> bool,
/// ) -> Result<(), papo::Error> {
///     if let Some(peer) = current_peer(client, nick, stored, &mut accept).await? {
///         match client.privmsg(&peer, "release approved").await {
///             Ok(_) => {}
///             Err(papo::Error::TargetGone) => {
///                 if let Some(rebound) = current_peer(client, nick, stored, &mut accept).await? {
///                     client.privmsg(&rebound, "release approved").await?;
///                 }
///             }
///             Err(error) => return Err(error),
///         }
///     }
///     Ok(())
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

    /// Reports whether this pin is backed by a proven account at an authenticated server scope.
    #[must_use]
    pub const fn is_verified(&self) -> bool {
        matches!(self.0, PeerIdentity::Account(_))
    }

    /// Returns the stable representation of a verified pin.
    ///
    /// Session pins return `None` so they cannot accidentally be stored as durable identities.
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
