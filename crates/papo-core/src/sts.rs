//! Strict Transport Security policy parsing and storage.

use std::time::{Duration, SystemTime};

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
        /// The wall-clock time at which the policy expires.
        expires_at: SystemTime,
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
/// `now` is supplied by the caller so policy logic never reads a clock.
#[must_use]
pub fn decide(
    directive: StsDirective,
    connection: ConnectionSecurity,
    host: &str,
    now: SystemTime,
) -> StsAction {
    match connection {
        ConnectionSecurity::Plaintext => directive
            .port
            .map_or(StsAction::Ignore, |port| StsAction::UpgradeTo { port }),
        ConnectionSecurity::Secure => match directive.duration {
            Some(duration) if duration.is_zero() => StsAction::Remove {
                host: host.to_owned(),
            },
            Some(duration) => now
                .checked_add(duration)
                .map_or(StsAction::Ignore, |expires_at| StsAction::Persist {
                    host: host.to_owned(),
                    expires_at,
                }),
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{ConnectionSecurity, StsAction, StsDirective, StsParseError, StsPolicy, decide};

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
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let with_port = StsPolicy::parse("port=6697,duration=31536000").unwrap();
        assert_eq!(
            decide(
                with_port,
                ConnectionSecurity::Plaintext,
                "irc.example.net",
                now
            ),
            StsAction::UpgradeTo { port: 6697 }
        );
        let duration_only = StsPolicy::parse("duration=31536000").unwrap();
        assert_eq!(
            decide(
                duration_only,
                ConnectionSecurity::Plaintext,
                "irc.example.net",
                now
            ),
            StsAction::Ignore
        );
    }

    #[test]
    fn persists_or_removes_only_from_a_secure_connection() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let duration = StsPolicy::parse("duration=15552000,preload=consent").unwrap();
        let StsAction::Persist { host, expires_at } =
            decide(duration, ConnectionSecurity::Secure, "irc.example.net", now)
        else {
            panic!("duration did not produce a persistent policy");
        };
        assert_eq!(host, "irc.example.net");
        assert_eq!(
            expires_at.duration_since(now).unwrap().as_secs(),
            15_552_000
        );
        let remove = StsPolicy::parse("duration=0").unwrap();
        assert_eq!(
            decide(remove, ConnectionSecurity::Secure, "irc.example.net", now),
            StsAction::Remove {
                host: "irc.example.net".to_owned(),
            }
        );
    }

    #[test]
    fn ignores_a_secure_policy_with_only_a_port() {
        let directive = StsPolicy::parse("port=6697").unwrap();
        assert_eq!(
            decide(
                directive,
                ConnectionSecurity::Secure,
                "irc.example.net",
                UNIX_EPOCH
            ),
            StsAction::Ignore
        );
    }
}
