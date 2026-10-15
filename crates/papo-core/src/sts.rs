//! Strict Transport Security policy parsing and storage.

use std::{
    collections::HashMap,
    time::{Duration, SystemTime},
};

/// Stateless operations for `IRCv3` Strict Transport Security policies.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StsPolicy;

impl StsPolicy {
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    UpgradeTo {
        /// The advertised TLS port.
        port: u16,
    },
    /// Persist a secure-only policy for a hostname.
    Persist {
        /// The hostname used to establish the connection.
        host: String,
        /// The duration for which the policy should remain active.
        duration: Duration,
    },
    /// Remove the stored policy for a hostname.
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
pub struct StoredPolicy {
    /// The hostname to which this policy applies.
    pub host: String,
    /// The wall-clock time at which this policy stops requiring TLS.
    pub expires_at: SystemTime,
    /// The duration used to refresh expiry when a connection closes.
    pub duration: Duration,
}

impl StoredPolicy {
    /// Reports whether this policy still requires TLS at `now`.
    #[must_use]
    pub fn is_active_at(&self, now: SystemTime) -> bool {
        now < self.expires_at
    }
}

/// Persistent storage for per-host STS policies.
pub trait StsStore {
    /// Returns the stored policy for `host`, if one exists.
    fn get(&self, host: &str) -> Option<StoredPolicy>;

    /// Inserts or replaces a host's policy.
    fn put(&mut self, policy: StoredPolicy);

    /// Removes a host's policy, if one exists.
    fn remove(&mut self, host: &str);
}

/// An in-memory STS policy store.
#[derive(Clone, Debug, Default)]
pub struct MemoryStsStore {
    policies: HashMap<String, StoredPolicy>,
}

impl MemoryStsStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl StsStore for MemoryStsStore {
    fn get(&self, host: &str) -> Option<StoredPolicy> {
        self.policies.get(&normalize_host(host)).cloned()
    }

    fn put(&mut self, mut policy: StoredPolicy) {
        policy.host = normalize_host(&policy.host);
        self.policies.insert(policy.host.clone(), policy);
    }

    fn remove(&mut self, host: &str) {
        self.policies.remove(&normalize_host(host));
    }
}

fn normalize_host(host: &str) -> String {
    host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase()
}

/// Refreshes a stored policy from the time its connection closed.
#[must_use]
pub fn reschedule_on_disconnect(stored: StoredPolicy, disconnected_at: SystemTime) -> StoredPolicy {
    let expires_at = disconnected_at
        .checked_add(stored.duration)
        .unwrap_or(stored.expires_at);
    StoredPolicy {
        expires_at,
        ..stored
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{
        ConnectionSecurity, MemoryStsStore, StoredPolicy, StsAction, StsDirective, StsParseError,
        StsPolicy, StsStore, decide, reschedule_on_disconnect,
    };

    #[test]
    fn parses_every_policy_part() {
        let directive = StsPolicy::parse("port=6697,duration=15552000,preload").unwrap();
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
            StsPolicy::parse("unknown,preload=yes,foo=bar").unwrap(),
            StsDirective {
                preload: true,
                ..StsDirective::default()
            }
        );
    }

    #[test]
    fn accepts_an_empty_policy_as_having_no_directives() {
        assert_eq!(StsPolicy::parse("").unwrap(), StsDirective::default());
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
            assert_eq!(StsPolicy::parse(value), Err(error));
        }
    }

    #[test]
    fn upgrades_plaintext_only_when_a_port_is_present() {
        let with_port = StsPolicy::parse("port=6697,duration=31536000").unwrap();
        assert_eq!(
            decide(with_port, ConnectionSecurity::Plaintext, "irc.example.net"),
            StsAction::UpgradeTo { port: 6697 }
        );
        let duration_only = StsPolicy::parse("duration=31536000").unwrap();
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
        let duration = StsPolicy::parse("duration=15552000,preload=consent").unwrap();
        let StsAction::Persist { host, duration } =
            decide(duration, ConnectionSecurity::Secure, "IRC.Example.NET.")
        else {
            panic!("duration did not produce a persistent policy");
        };
        assert_eq!(host, "irc.example.net");
        assert_eq!(duration, Duration::from_hours(4_320));
        let remove = StsPolicy::parse("duration=0").unwrap();
        assert_eq!(
            decide(remove, ConnectionSecurity::Secure, "IRC.Example.NET."),
            StsAction::Remove {
                host: "irc.example.net".to_owned(),
            }
        );
    }

    #[test]
    fn ignores_a_secure_policy_with_only_a_port() {
        let directive = StsPolicy::parse("port=6697").unwrap();
        assert_eq!(
            decide(directive, ConnectionSecurity::Secure, "irc.example.net"),
            StsAction::Ignore
        );
    }

    #[test]
    fn stores_replaces_and_removes_policies() {
        let mut store = MemoryStsStore::new();
        let policy = StoredPolicy {
            host: "irc.example.net".to_owned(),
            expires_at: UNIX_EPOCH + Duration::from_secs(2_000_000),
            duration: Duration::from_secs(60),
        };
        assert_eq!(store.get("irc.example.net"), None);
        store.put(policy.clone());
        assert_eq!(store.get("irc.example.net"), Some(policy));

        let replacement = StoredPolicy {
            host: "irc.example.net".to_owned(),
            expires_at: UNIX_EPOCH + Duration::from_secs(3_000_001),
            duration: Duration::from_secs(120),
        };
        store.put(replacement.clone());
        assert_eq!(store.get("irc.example.net"), Some(replacement));
        store.remove("irc.example.net");
        assert_eq!(store.get("irc.example.net"), None);
    }

    #[test]
    fn normalizes_hostnames_at_the_store_boundary() {
        let mut store = MemoryStsStore::new();
        store.put(StoredPolicy {
            host: "irc.Example.com".to_owned(),
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
        let policy = StoredPolicy {
            host: "irc.example.net".to_owned(),
            expires_at: now - Duration::from_secs(1),
            duration: Duration::from_secs(60),
        };
        assert!(!policy.is_active_at(now));
        assert!(!policy.is_active_at(policy.expires_at));
        assert!(policy.is_active_at(policy.expires_at - Duration::from_secs(1)));
    }

    #[test]
    fn reschedules_expiry_from_disconnect_time() {
        let duration = StsPolicy::parse("duration=2592000")
            .unwrap()
            .duration
            .unwrap();
        let stored = StoredPolicy {
            host: "irc.example.net".to_owned(),
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
