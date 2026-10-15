#[allow(dead_code)]
pub(crate) mod scram;

/// Ordered SASL mechanisms to try during connection registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sasl {
    pub(crate) mechanisms: Vec<Mechanism>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Mechanism {
    Plain { username: String, password: String },
    External { authzid: Option<String> },
    ScramSha256 { username: String, password: String },
}

impl Sasl {
    /// Uses the SASL PLAIN mechanism.
    #[must_use]
    pub fn plain(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            mechanisms: vec![Mechanism::Plain {
                username: username.into(),
                password: password.into(),
            }],
        }
    }

    /// Uses the SASL EXTERNAL mechanism with an empty authorization identity.
    #[must_use]
    pub fn external() -> Self {
        Self {
            mechanisms: vec![Mechanism::External { authzid: None }],
        }
    }

    /// Sets the authorization identity sent by SASL EXTERNAL.
    #[must_use]
    pub fn authzid(mut self, authzid: impl Into<String>) -> Self {
        for mechanism in &mut self.mechanisms {
            if let Mechanism::External { authzid: current } = mechanism {
                *current = Some(authzid.into());
                break;
            }
        }
        self
    }

    /// Uses the SASL SCRAM-SHA-256 mechanism.
    #[must_use]
    pub fn scram_sha256(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            mechanisms: vec![Mechanism::ScramSha256 {
                username: username.into(),
                password: password.into(),
            }],
        }
    }

    /// Prefers SCRAM-SHA-256 and falls back to PLAIN.
    #[must_use]
    pub fn any(username: impl Into<String>, password: impl Into<String>) -> Self {
        let username = username.into();
        let password = password.into();
        Self {
            mechanisms: vec![
                Mechanism::ScramSha256 {
                    username: username.clone(),
                    password: password.clone(),
                },
                Mechanism::Plain { username, password },
            ],
        }
    }
}

/// Source of entropy for authentication mechanisms that require random bytes.
pub trait Entropy {
    /// Fills `buf` with unpredictable bytes.
    fn fill(&mut self, buf: &mut [u8]);
}
