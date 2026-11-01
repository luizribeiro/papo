use std::{fmt, hash::Hash, str::FromStr};

const PREFIX: &str = "papo-peer-v1:";

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

    fn legacy(&self) -> String {
        let mut value = format!("server={}:{}", self.server.len(), self.server);
        if let Some(bouncer) = &self.bouncer {
            use std::fmt::Write as _;
            write!(
                value,
                ";login={}:{};network={}:{};upstream={}:{}",
                bouncer.login.len(),
                bouncer.login,
                bouncer.network.len(),
                bouncer.network,
                bouncer.upstream.len(),
                bouncer.upstream
            )
            .expect("writing to a string cannot fail");
        }
        value
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
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct PeerKey(PeerIdentity);

#[derive(Clone, Eq, Hash, PartialEq)]
enum PeerIdentity {
    Account(String),
    Continuity(papo_core::PeerToken),
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
        Self::verified_legacy(&scope.legacy(), casemapping, account)
    }

    fn verified_legacy(scope: &str, casemapping: crate::Casemapping, account: &str) -> Self {
        let value = format!(
            "{PREFIX}{}:{}:{scope}{account}",
            casemapping.as_str(),
            scope.len()
        );
        Self(PeerIdentity::Account(value))
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
            PeerIdentity::Account(value) => Some(value),
            PeerIdentity::Continuity(_) => None,
        }
    }
}

impl FromStr for PeerKey {
    type Err = InvalidPeerKey;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (casemapping, rest) = value
            .strip_prefix(PREFIX)
            .and_then(|value| value.split_once(':'))
            .ok_or(InvalidPeerKey)?;
        let casemapping = casemapping
            .parse::<crate::Casemapping>()
            .map_err(|_| InvalidPeerKey)?;
        let (scope_len, joined) = rest.split_once(':').ok_or(InvalidPeerKey)?;
        let scope_len = scope_len.parse::<usize>().map_err(|_| InvalidPeerKey)?;
        let (scope, account) = joined
            .get(..scope_len)
            .zip(joined.get(scope_len..))
            .ok_or(InvalidPeerKey)?;
        if scope.is_empty() || account.is_empty() {
            return Err(InvalidPeerKey);
        }
        let account = casemapping.account(account);
        account.check().map_err(|_| InvalidPeerKey)?;
        Ok(Self::verified_legacy(scope, casemapping, account.key()))
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

impl std::error::Error for InvalidPeerKey {}
