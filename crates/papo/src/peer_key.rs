use std::{
    fmt,
    hash::Hash,
    sync::atomic::{AtomicU64, Ordering},
};

const PREFIX: &str = "papo-peer-v1:";
static NEXT_EPHEMERAL: AtomicU64 = AtomicU64::new(1);

/// The verified account or session-scoped identity behind a direct conversation.
///
/// Verified keys compare the account under the server's advertised casemapping together with the
/// server name authenticated by TLS. A bound bouncer network ID is part of that scope. They remain
/// equal across reconnects, process restarts, and multiple simultaneous nicknames logged into the
/// same account. Connecting through another host deliberately produces a different key.
///
/// Plaintext connections and caller-supplied streams produce only session keys because papo did
/// not verify a certificate for their server name. `ISUPPORT NETWORK` is descriptive and never
/// affects identity. Session keys remain equal only while papo can prove continuity of one IRC
/// user. They change after reconnect, departure and nickname reuse, or account logout.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct PeerKey(PeerIdentity);

#[derive(Clone, Eq, Hash, PartialEq)]
enum PeerIdentity {
    Verified(String),
    Tracked(papo_core::PeerToken),
    Ephemeral {
        origin: crate::Origin,
        sequence: u64,
    },
}

impl PeerKey {
    pub(crate) fn live(
        scope: Option<&str>,
        account: &crate::Account,
        token: papo_core::PeerToken,
        origin: crate::Origin,
    ) -> Self {
        Self::new(scope, account, token.__is_known().then_some(token), origin)
    }

    #[allow(dead_code)]
    pub(crate) fn historical(
        scope: Option<&str>,
        account: &crate::Account,
        origin: crate::Origin,
    ) -> Self {
        Self::new(scope, account, None, origin)
    }

    fn new(
        scope: Option<&str>,
        account: &crate::Account,
        tracked: Option<papo_core::PeerToken>,
        origin: crate::Origin,
    ) -> Self {
        if let (Some(scope), crate::Account::LoggedIn(account)) = (scope, account)
            && !scope.is_empty()
        {
            return Self::verified(scope, account.casemapping(), account.key());
        }
        tracked.map_or_else(
            || Self::ephemeral(origin),
            |token| Self(PeerIdentity::Tracked(token)),
        )
    }

    fn verified(scope: &str, casemapping: crate::Casemapping, account: &str) -> Self {
        let value = format!(
            "{PREFIX}{}:{}:{scope}{account}",
            casemapping.as_str(),
            scope.len()
        );
        Self(PeerIdentity::Verified(value))
    }

    fn ephemeral(origin: crate::Origin) -> Self {
        let mut sequence = NEXT_EPHEMERAL.load(Ordering::Relaxed);
        loop {
            let next = sequence.checked_add(1).expect("peer key space exhausted");
            match NEXT_EPHEMERAL.compare_exchange_weak(
                sequence,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => sequence = actual,
            }
        }
        Self(PeerIdentity::Ephemeral { origin, sequence })
    }

    /// Reports whether this key is backed by a proven account at an authenticated server scope.
    #[must_use]
    pub const fn is_verified(&self) -> bool {
        matches!(self.0, PeerIdentity::Verified(_))
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
