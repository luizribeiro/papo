#[allow(dead_code)]
pub(crate) mod scram;

use std::{collections::VecDeque, time::Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use papo_proto::MessageRef;

use crate::{Config, Handled, Outbox};

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

pub(super) struct SaslSession {
    configured: Vec<Mechanism>,
    remaining: VecDeque<Mechanism>,
    active: Option<Mechanism>,
    awaiting_challenge: bool,
    finished: bool,
    account: Option<String>,
    device: Option<String>,
    #[allow(dead_code)]
    entropy: Option<Box<dyn Entropy + Send>>,
}

impl std::fmt::Debug for SaslSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SaslSession")
            .field("configured", &self.configured)
            .field("remaining", &self.remaining)
            .field("active", &self.active)
            .field("awaiting_challenge", &self.awaiting_challenge)
            .field("finished", &self.finished)
            .field("account", &self.account)
            .field("device", &self.device)
            .field("has_entropy", &self.entropy.is_some())
            .finish()
    }
}

impl SaslSession {
    pub(super) fn new(config: &Config, entropy: Option<Box<dyn Entropy + Send>>) -> Self {
        let configured = config
            .sasl
            .as_ref()
            .map_or_else(Vec::new, |sasl| sasl.mechanisms.clone());
        Self {
            remaining: configured.clone().into(),
            active: None,
            awaiting_challenge: false,
            finished: configured.is_empty(),
            configured,
            account: None,
            device: config.device.clone(),
            entropy,
        }
    }

    pub(super) fn handle_connected(&mut self) {
        self.remaining = self.configured.clone().into();
        self.active = None;
        self.awaiting_challenge = false;
        self.finished = self.configured.is_empty();
        self.account = None;
    }

    pub(super) fn handle_cap_ack(
        &mut self,
        _now: Instant,
        advertised: Option<&str>,
        outbox: &mut Outbox,
    ) {
        if self.finished {
            return;
        }
        if let Some(advertised) = advertised {
            self.remaining.retain(|mechanism| {
                advertised
                    .split(',')
                    .any(|name| name.eq_ignore_ascii_case(mechanism.name()))
            });
        }
        self.start_next(outbox);
    }

    pub(super) fn handle_message(
        &mut self,
        _now: Instant,
        message: &MessageRef<'_>,
        outbox: &mut Outbox,
    ) -> Handled {
        if message.command_is("900") {
            if let Some(account) = message.param(2) {
                self.account = Some(account.to_owned());
            }
            return Handled::Yes;
        }
        if message.command_is("901") {
            self.account = None;
            return Handled::Yes;
        }
        if message.command_is("903") || message.command_is("907") {
            self.active = None;
            self.awaiting_challenge = false;
            self.finished = true;
            return Handled::Yes;
        }
        if !message.command_is("AUTHENTICATE") {
            return Handled::No;
        }
        let Some(Mechanism::Plain { username, password }) = self.active.as_ref() else {
            return Handled::No;
        };
        if !self.awaiting_challenge || message.param(0) != Some("+") {
            return Handled::No;
        }
        let authcid = with_device(username, self.device.as_deref());
        let response = STANDARD.encode(format!("\0{authcid}\0{password}"));
        outbox.transmit("AUTHENTICATE", [&response]);
        self.awaiting_challenge = false;
        Handled::Yes
    }

    pub(super) fn unavailable(&mut self, reason: &str, outbox: &mut Outbox) {
        if self.finished {
            return;
        }
        self.finished = true;
        let _ = (reason, outbox);
    }

    pub(super) const fn is_finished(&self) -> bool {
        self.finished
    }

    pub(super) fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    fn start_next(&mut self, outbox: &mut Outbox) {
        let Some(mechanism) = self.remaining.pop_front() else {
            return self.unavailable("server offers none of the configured mechanisms", outbox);
        };
        outbox.transmit("AUTHENTICATE", [mechanism.name()]);
        self.active = Some(mechanism);
        self.awaiting_challenge = true;
    }
}

fn with_device(username: &str, device: Option<&str>) -> String {
    device.map_or_else(
        || username.to_owned(),
        |device| format!("{username}@{device}"),
    )
}

impl Mechanism {
    fn name(&self) -> &'static str {
        match self {
            Self::Plain { .. } => "PLAIN",
            Self::External { .. } => "EXTERNAL",
            Self::ScramSha256 { .. } => "SCRAM-SHA-256",
        }
    }
}
