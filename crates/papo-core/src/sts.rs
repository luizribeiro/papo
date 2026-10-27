//! Strict Transport Security policy parsing and storage.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

impl StsDirective {
    /// Parses an `sts` capability value.
    ///
    /// Unknown policy keys are ignored as required by the `IRCv3` specification.
    ///
    /// # Errors
    ///
    /// Returns [`StsParseError`] when a recognized key is duplicated or its
    /// value cannot be parsed.
    pub fn parse(cap_value: &str) -> Result<StsDirective, StsParseError> {
        let mut directive = StsDirective::default();
        for token in cap_value.split(',') {
            let (key, value) = token
                .split_once('=')
                .map_or((token, None), |(key, value)| (key, Some(value)));
            match key {
                "port" => {
                    if directive.port.is_some() {
                        return Err(StsParseError::DuplicateKey("port"));
                    }
                    let port = value
                        .ok_or(StsParseError::InvalidPort)?
                        .parse()
                        .map_err(|_| StsParseError::InvalidPort)?;
                    if port == 0 {
                        return Err(StsParseError::InvalidPort);
                    }
                    directive.port = Some(port);
                }
                "duration" => {
                    if directive.duration.is_some() {
                        return Err(StsParseError::DuplicateKey("duration"));
                    }
                    let seconds = value
                        .ok_or(StsParseError::InvalidDuration)?
                        .parse()
                        .map_err(|_| StsParseError::InvalidDuration)?;
                    directive.duration = Some(Duration::from_secs(seconds));
                }
                "preload" => {
                    if directive.preload {
                        return Err(StsParseError::DuplicateKey("preload"));
                    }
                    directive.preload = true;
                }
                _ => {}
            }
        }
        Ok(directive)
    }
}

/// Whether the current connection is protected by TLS.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ConnectionSecurity {
    /// A connection whose peer and confidentiality were verified with TLS.
    Secure,
    /// A connection without TLS protection.
    Plaintext,
}

/// The action selected from an STS directive and the current connection.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StsAction {
    /// Reconnect to the same hostname using TLS on this port.
    #[non_exhaustive]
    UpgradeTo {
        /// The advertised TLS port.
        port: u16,
    },
    /// Persist a secure-only policy for a hostname.
    #[non_exhaustive]
    Persist {
        /// The hostname used to establish the connection.
        host: String,
        /// The duration for which the policy should remain active.
        duration: Duration,
    },
    /// Remove the stored policy for a hostname.
    #[non_exhaustive]
    Remove {
        /// The hostname used to establish the connection.
        host: String,
    },
    /// Take no action because the applicable directive is absent or invalid.
    Ignore,
}

/// Selects the action for an STS directive received on `connection`.
///
#[must_use]
pub fn decide(directive: StsDirective, connection: ConnectionSecurity, host: &str) -> StsAction {
    match connection {
        ConnectionSecurity::Plaintext => directive
            .port
            .map_or(StsAction::Ignore, |port| StsAction::UpgradeTo { port }),
        ConnectionSecurity::Secure => match directive.duration {
            Some(duration) if duration.is_zero() => StsAction::Remove {
                host: normalize_host(host),
            },
            Some(duration) => StsAction::Persist {
                host: normalize_host(host),
                duration,
            },
            None => StsAction::Ignore,
        },
    }
}

/// The recognized parts of an `IRCv3` STS policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct StsDirective {
    /// The TLS port advertised to plaintext clients.
    pub port: Option<u16>,
    /// The persistence duration advertised to secure clients.
    pub duration: Option<Duration>,
    /// Whether the server consents to inclusion in preload lists.
    pub preload: bool,
}

/// An invalid recognized field in an STS capability value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StsParseError {
    /// The port is missing or is not an unsigned 16-bit integer.
    InvalidPort,
    /// The duration is missing or is not an unsigned 64-bit integer.
    InvalidDuration,
    /// A key that may occur once was repeated.
    DuplicateKey(&'static str),
}

/// A persisted secure-only policy and the duration last advertised by its host.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct StsPolicy {
    /// The hostname to which this policy applies.
    pub host: String,
    /// The last TLS port used for this host; the policy itself applies to every port.
    pub port: u16,
    /// Whether `port` was learned over a WebSocket rather than TCP.
    pub websocket: bool,
    /// The wall-clock time at which this policy stops requiring TLS.
    pub expires_at: SystemTime,
    /// The duration used to refresh expiry when a connection closes.
    pub duration: Duration,
}

impl StsPolicy {
    /// Creates a persistent secure-only policy.
    ///
    /// The port is treated as learned over TCP. Use [`Self::websocket`] for a
    /// policy learned over a WebSocket.
    ///
    /// ```
    /// # use std::time::{Duration, SystemTime};
    /// # use papo_core as papo;
    /// let policy = papo::sts::StsPolicy::new(
    ///     "irc.example.net",
    ///     6697,
    ///     SystemTime::now() + Duration::from_secs(3600),
    ///     Duration::from_secs(3600),
    /// );
    /// # _ = policy;
    /// ```
    #[must_use]
    pub fn new(
        host: impl Into<String>,
        port: u16,
        expires_at: SystemTime,
        duration: Duration,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            websocket: false,
            expires_at,
            duration,
        }
    }

    /// Records whether this policy's port was learned over a WebSocket.
    #[must_use]
    pub const fn websocket(mut self, websocket: bool) -> Self {
        self.websocket = websocket;
        self
    }

    /// Reports whether this policy still requires TLS at `now`.
    #[must_use]
    pub fn is_active_at(&self, now: SystemTime) -> bool {
        now < self.expires_at
    }
}

/// Storage for per-host STS policies.
///
/// The in-memory store forgets TLS-only networks when the app restarts.
/// Applications that need protection across restarts can persist policies in
/// platform-appropriate storage. The repository's
/// [file-backed example](https://github.com/luizribeiro/papo/tree/main/examples/sts-store)
/// shows one way for an app to own that persistence.
///
/// Methods take shared references so one store can be wrapped in [`Arc`] and used by multiple
/// clients. Implementations synchronize mutable storage internally.
pub trait StsStore: Send + Sync {
    /// Returns the stored policy for `host`, if one exists.
    ///
    /// Callers provide an ASCII-lowercase hostname without a trailing root dot.
    fn get(&self, host: &str) -> Option<StsPolicy>;

    /// Inserts or replaces a host's policy.
    ///
    /// Callers normalize [`StsPolicy::host`] to ASCII lowercase without a
    /// trailing root dot.
    fn put(&self, policy: StsPolicy);

    /// Removes a host's policy, if one exists.
    ///
    /// Callers provide an ASCII-lowercase hostname without a trailing root dot.
    fn remove(&self, host: &str);
}

impl<T: StsStore + ?Sized> StsStore for Arc<T> {
    fn get(&self, host: &str) -> Option<StsPolicy> {
        T::get(self, host)
    }

    fn put(&self, policy: StsPolicy) {
        T::put(self, policy);
    }

    fn remove(&self, host: &str) {
        T::remove(self, host);
    }
}

/// An in-memory STS policy store.
#[derive(Clone, Debug, Default)]
pub struct MemoryStsStore {
    policies: Arc<Mutex<HashMap<String, StsPolicy>>>,
}

impl MemoryStsStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl StsStore for MemoryStsStore {
    fn get(&self, host: &str) -> Option<StsPolicy> {
        self.policies
            .lock()
            .unwrap()
            .get(&normalize_host(host))
            .cloned()
    }

    fn put(&self, mut policy: StsPolicy) {
        policy.host = normalize_host(&policy.host);
        self.policies
            .lock()
            .unwrap()
            .insert(policy.host.clone(), policy);
    }

    fn remove(&self, host: &str) {
        self.policies.lock().unwrap().remove(&normalize_host(host));
    }
}

fn normalize_host(host: &str) -> String {
    host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase()
}

/// Refreshes a stored policy from the time its connection closed.
#[must_use]
pub fn reschedule_on_disconnect(stored: StsPolicy, disconnected_at: SystemTime) -> StsPolicy {
    let expires_at = disconnected_at
        .checked_add(stored.duration)
        .unwrap_or(stored.expires_at);
    StsPolicy {
        expires_at,
        ..stored
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{Duration, UNIX_EPOCH},
    };

    use super::{
        ConnectionSecurity, MemoryStsStore, StsAction, StsDirective, StsParseError, StsPolicy,
        StsStore, decide, reschedule_on_disconnect,
    };

    #[test]
    fn parses_every_policy_part() {
        let directive = StsDirective::parse("port=6697,duration=15552000,preload").unwrap();
        assert_eq!(directive.port, Some(6697));
        assert_eq!(
            directive.duration.map(|value| value.as_secs()),
            Some(15_552_000)
        );
        assert!(directive.preload);
    }

    #[test]
    fn ignores_unknown_keys_and_preload_values() {
        assert_eq!(
            StsDirective::parse("unknown,preload=yes,foo=bar").unwrap(),
            StsDirective {
                preload: true,
                ..StsDirective::default()
            }
        );
    }

    #[test]
    fn accepts_an_empty_policy_as_having_no_directives() {
        assert_eq!(StsDirective::parse("").unwrap(), StsDirective::default());
    }

    #[test]
    fn rejects_invalid_or_duplicate_values() {
        for (value, error) in [
            ("port", StsParseError::InvalidPort),
            ("port=not-a-port", StsParseError::InvalidPort),
            ("port=0", StsParseError::InvalidPort),
            ("port=65536", StsParseError::InvalidPort),
            ("duration", StsParseError::InvalidDuration),
            ("duration=-1", StsParseError::InvalidDuration),
            ("port=6697,port=6698", StsParseError::DuplicateKey("port")),
            (
                "duration=60,duration=120",
                StsParseError::DuplicateKey("duration"),
            ),
            ("preload,preload", StsParseError::DuplicateKey("preload")),
        ] {
            assert_eq!(StsDirective::parse(value), Err(error));
        }
    }

    #[test]
    fn upgrades_plaintext_only_when_a_port_is_present() {
        let with_port = StsDirective::parse("port=6697,duration=31536000").unwrap();
        assert_eq!(
            decide(with_port, ConnectionSecurity::Plaintext, "irc.example.net"),
            StsAction::UpgradeTo { port: 6697 }
        );
        let duration_only = StsDirective::parse("duration=31536000").unwrap();
        assert_eq!(
            decide(
                duration_only,
                ConnectionSecurity::Plaintext,
                "irc.example.net"
            ),
            StsAction::Ignore
        );
    }

    #[test]
    fn persists_or_removes_only_from_a_secure_connection() {
        let duration = StsDirective::parse("duration=15552000,preload=consent").unwrap();
        let StsAction::Persist { host, duration } =
            decide(duration, ConnectionSecurity::Secure, "IRC.Example.NET.")
        else {
            panic!("duration did not produce a persistent policy");
        };
        assert_eq!(host, "irc.example.net");
        assert_eq!(duration, Duration::from_hours(4_320));
        let remove = StsDirective::parse("duration=0").unwrap();
        assert_eq!(
            decide(remove, ConnectionSecurity::Secure, "IRC.Example.NET."),
            StsAction::Remove {
                host: "irc.example.net".to_owned(),
            }
        );
    }

    #[test]
    fn ignores_a_secure_policy_with_only_a_port() {
        let directive = StsDirective::parse("port=6697").unwrap();
        assert_eq!(
            decide(directive, ConnectionSecurity::Secure, "irc.example.net"),
            StsAction::Ignore
        );
    }

    #[test]
    fn stores_replaces_and_removes_policies() {
        let store = MemoryStsStore::new();
        let policy = StsPolicy {
            host: "irc.example.net".to_owned(),
            port: 6697,
            websocket: false,
            expires_at: UNIX_EPOCH + Duration::from_secs(2_000_000),
            duration: Duration::from_secs(60),
        };
        assert_eq!(store.get("irc.example.net"), None);
        store.put(policy.clone());
        assert_eq!(store.get("irc.example.net"), Some(policy));

        let replacement = StsPolicy {
            host: "irc.example.net".to_owned(),
            port: 6698,
            websocket: false,
            expires_at: UNIX_EPOCH + Duration::from_secs(3_000_001),
            duration: Duration::from_secs(120),
        };
        store.put(replacement.clone());
        assert_eq!(store.get("irc.example.net"), Some(replacement));
        store.remove("irc.example.net");
        assert_eq!(store.get("irc.example.net"), None);
    }

    #[test]
    fn arc_stores_share_policy_updates() {
        let store = Arc::new(MemoryStsStore::new());
        let shared = Arc::clone(&store);
        let policy = StsPolicy::new(
            "irc.example.net",
            6697,
            UNIX_EPOCH + Duration::from_secs(2_000_000),
            Duration::from_secs(60),
        );

        <Arc<MemoryStsStore> as StsStore>::put(&store, policy.clone());

        assert_eq!(shared.get("irc.example.net"), Some(policy));
    }

    #[test]
    fn normalizes_hostnames_at_the_store_boundary() {
        let store = MemoryStsStore::new();
        store.put(StsPolicy {
            host: "irc.Example.com".to_owned(),
            port: 6697,
            websocket: false,
            expires_at: UNIX_EPOCH + Duration::from_secs(2_000_000),
            duration: Duration::from_secs(60),
        });

        let policy = store.get("IRC.example.com.").unwrap();
        assert_eq!(policy.host, "irc.example.com");
        store.remove("IRC.example.com.");
        assert_eq!(store.get("irc.example.com"), None);
    }

    #[test]
    fn an_expired_stored_policy_is_inactive() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let policy = StsPolicy {
            host: "irc.example.net".to_owned(),
            port: 6697,
            websocket: false,
            expires_at: now - Duration::from_secs(1),
            duration: Duration::from_secs(60),
        };
        assert!(!policy.is_active_at(now));
        assert!(!policy.is_active_at(policy.expires_at));
        assert!(policy.is_active_at(policy.expires_at - Duration::from_secs(1)));
    }

    #[test]
    fn reschedules_expiry_from_disconnect_time() {
        let duration = StsDirective::parse("duration=2592000")
            .unwrap()
            .duration
            .unwrap();
        let stored = StsPolicy {
            host: "irc.example.net".to_owned(),
            port: 6697,
            websocket: false,
            expires_at: UNIX_EPOCH + duration,
            duration,
        };
        let disconnected_at = UNIX_EPOCH + Duration::from_hours(48);
        let refreshed = reschedule_on_disconnect(stored, disconnected_at);

        assert_eq!(refreshed.host, "irc.example.net");
        assert_eq!(refreshed.duration.as_secs(), 2_592_000);
        assert_eq!(
            refreshed
                .expires_at
                .duration_since(disconnected_at)
                .unwrap()
                .as_secs(),
            2_592_000
        );
    }
}
