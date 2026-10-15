//! Strict Transport Security policy parsing and storage.

use std::time::Duration;

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
    use super::{StsDirective, StsParseError, StsPolicy};

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
}
