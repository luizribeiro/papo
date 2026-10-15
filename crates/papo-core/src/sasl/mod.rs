#[allow(dead_code)]
pub(crate) mod scram;

use std::{collections::VecDeque, time::Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use papo_proto::MessageRef;

use crate::{Config, Handled, Outbox};

const SASL_CHUNK_LEN: usize = 400;

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
    completion: Completion,
    account: Option<String>,
    device: Option<String>,
    challenge: String,
    entropy: Option<Box<dyn Entropy + Send>>,
    require: bool,
    failures: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Completion {
    Pending,
    Success,
    Failed,
}

impl std::fmt::Debug for SaslSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SaslSession")
            .field("configured", &self.configured)
            .field("remaining", &self.remaining)
            .field("active", &self.active)
            .field("awaiting_challenge", &self.awaiting_challenge)
            .field("completion", &self.completion)
            .field("account", &self.account)
            .field("device", &self.device)
            .field("challenge_len", &self.challenge.len())
            .field("has_entropy", &self.entropy.is_some())
            .field("require", &self.require)
            .field("failures", &self.failures)
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
            completion: if configured.is_empty() {
                Completion::Success
            } else {
                Completion::Pending
            },
            configured,
            account: None,
            device: config.device.clone(),
            challenge: String::new(),
            entropy,
            require: config.require_sasl,
            failures: Vec::new(),
        }
    }

    pub(super) fn handle_connected(&mut self) {
        self.remaining = self.configured.clone().into();
        self.active = None;
        self.awaiting_challenge = false;
        self.completion = if self.configured.is_empty() {
            Completion::Success
        } else {
            Completion::Pending
        };
        self.account = None;
        self.challenge.clear();
        self.failures.clear();
    }

    pub(super) fn handle_cap_ack(
        &mut self,
        _now: Instant,
        advertised: Option<&str>,
        outbox: &mut Outbox,
    ) {
        if self.completion != Completion::Pending {
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
            self.completion = Completion::Success;
            return Handled::Yes;
        }
        if message.command_is("904") || message.command_is("905") {
            self.record_active_failure(message.param(1).unwrap_or("SASL mechanism failed"));
            self.start_next(outbox);
            return Handled::Yes;
        }
        if message.command_is("908") {
            if let Some(available) = message.param(1) {
                self.retain_available(available);
            }
            return Handled::Yes;
        }
        if message.command_is("902") {
            self.fail_all(message.param(1).unwrap_or("account is unavailable"), outbox);
            return Handled::Yes;
        }
        if message.command_is("906") {
            self.fail_all(
                message.param(1).unwrap_or("SASL exchange was aborted"),
                outbox,
            );
            return Handled::Yes;
        }
        if message.command_is("FAIL")
            && message.param(0) == Some("*")
            && message.param(1) == Some("ACCOUNT_REQUIRED")
        {
            self.completion = Completion::Failed;
            outbox.close(format!(
                "SASL failed: {}",
                message.param(2).unwrap_or("the server requires an account")
            ));
            return Handled::Yes;
        }
        if !message.command_is("AUTHENTICATE") {
            return Handled::No;
        }
        let Some(Mechanism::Plain { username, password }) = self.active.as_ref() else {
            return Handled::No;
        };
        let Some(chunk) = message.param(0).filter(|_| self.awaiting_challenge) else {
            return Handled::No;
        };
        let challenge = match receive_challenge(&mut self.challenge, chunk) {
            Ok(Some(challenge)) => challenge,
            Ok(None) => return Handled::Yes,
            Err(()) => {
                self.fail_all("the server sent an invalid SASL challenge", outbox);
                return Handled::Yes;
            }
        };
        let _ = challenge;
        let authcid = with_device(username, self.device.as_deref());
        transmit_response(format!("\0{authcid}\0{password}").as_bytes(), outbox);
        self.awaiting_challenge = false;
        Handled::Yes
    }

    pub(super) fn unavailable(&mut self, reason: &str, outbox: &mut Outbox) {
        if self.completion != Completion::Pending {
            return;
        }
        self.fail_all(reason, outbox);
    }

    pub(super) const fn is_finished(&self) -> bool {
        matches!(self.completion, Completion::Success)
            || (!self.require && matches!(self.completion, Completion::Failed))
    }

    pub(super) fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    fn start_next(&mut self, outbox: &mut Outbox) {
        loop {
            let Some(mechanism) = self.remaining.pop_front() else {
                return self.fail_all("every configured SASL mechanism failed", outbox);
            };
            if matches!(mechanism, Mechanism::ScramSha256 { .. }) && self.entropy.is_none() {
                self.failures.push(
                    "SCRAM-SHA-256 skipped because no entropy source was provided".to_owned(),
                );
                continue;
            }
            outbox.transmit("AUTHENTICATE", [mechanism.name()]);
            self.active = Some(mechanism);
            self.awaiting_challenge = true;
            self.challenge.clear();
            return;
        }
    }

    fn record_active_failure(&mut self, reason: &str) {
        if let Some(mechanism) = self.active.take() {
            self.failures
                .push(format!("{}: {reason}", mechanism.name()));
        }
        self.awaiting_challenge = false;
    }

    fn retain_available(&mut self, available: &str) {
        self.remaining.retain(|mechanism| {
            available
                .split(',')
                .any(|name| name.eq_ignore_ascii_case(mechanism.name()))
        });
    }

    fn fail_all(&mut self, reason: &str, outbox: &mut Outbox) {
        self.record_active_failure(reason);
        self.completion = Completion::Failed;
        self.failures.push(reason.to_owned());
        let reason = format!("SASL failed: {}", self.failures.join("; "));
        if self.require {
            outbox.close(reason);
        } else {
            outbox.event(crate::Event::AuthenticationFailed { reason });
        }
    }
}

fn with_device(username: &str, device: Option<&str>) -> String {
    device.map_or_else(
        || username.to_owned(),
        |device| format!("{username}@{device}"),
    )
}

fn receive_challenge(buffer: &mut String, chunk: &str) -> Result<Option<Vec<u8>>, ()> {
    if chunk != "+" {
        if chunk.len() > SASL_CHUNK_LEN {
            return Err(());
        }
        buffer.push_str(chunk);
        if chunk.len() == SASL_CHUNK_LEN {
            return Ok(None);
        }
    }
    let encoded = std::mem::take(buffer);
    STANDARD.decode(encoded).map(Some).map_err(|_| ())
}

fn transmit_response(response: &[u8], outbox: &mut Outbox) {
    let encoded = STANDARD.encode(response);
    if encoded.is_empty() {
        outbox.transmit("AUTHENTICATE", ["+"]);
        return;
    }
    for chunk in encoded.as_bytes().chunks(SASL_CHUNK_LEN) {
        outbox.transmit(
            "AUTHENTICATE",
            [std::str::from_utf8(chunk).expect("base64 is ASCII")],
        );
    }
    if encoded.len().is_multiple_of(SASL_CHUNK_LEN) {
        outbox.transmit("AUTHENTICATE", ["+"]);
    }
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

#[cfg(test)]
mod chunk_tests {
    use super::receive_challenge;

    #[test]
    fn reassembles_literal_400_byte_server_chunks() {
        let mut buffer = String::new();
        let first = "YQ==".repeat(100);
        assert_eq!(first.len(), 400);
        assert_eq!(receive_challenge(&mut buffer, &first), Ok(None));
        assert!(receive_challenge(&mut buffer, "+").is_err());

        let mut buffer = String::new();
        let first = "YWFh".repeat(100);
        assert_eq!(first.len(), 400);
        assert_eq!(receive_challenge(&mut buffer, &first), Ok(None));
        assert_eq!(
            receive_challenge(&mut buffer, "YQ==")
                .unwrap()
                .unwrap()
                .len(),
            301
        );
    }

    #[test]
    fn rejects_server_chunks_over_literal_400_bytes() {
        assert!(receive_challenge(&mut String::new(), &"A".repeat(401)).is_err());
    }
}
