pub(crate) mod scram;

use std::{collections::VecDeque, time::Duration};
use web_time::Instant;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use papo_proto::MessageRef;

use crate::{Config, Handled, Outbox, RedactedSecret, Secret};
use scram::ScramClient;

const SASL_CHUNK_LEN: usize = 400;
const MAX_SASL_CHALLENGE_LEN: usize = 4096;
const SASL_TIMEOUT: Duration = Duration::from_secs(30);

/// Ordered SASL mechanisms to try during connection registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sasl {
    pub(crate) mechanisms: Vec<Mechanism>,
    pub(crate) required: bool,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) enum Mechanism {
    Plain { username: String, password: Secret },
    OAuthBearer { token: Secret },
    External { authzid: Option<String> },
    ScramSha256 { username: String, password: Secret },
}

impl std::fmt::Debug for Mechanism {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plain { username, .. } => formatter
                .debug_struct("Plain")
                .field("username", username)
                .field("password", &RedactedSecret)
                .finish(),
            Self::OAuthBearer { .. } => formatter
                .debug_struct("OAuthBearer")
                .field("token", &RedactedSecret)
                .finish(),
            Self::External { authzid } => formatter
                .debug_struct("External")
                .field("authzid", authzid)
                .finish(),
            Self::ScramSha256 { username, .. } => formatter
                .debug_struct("ScramSha256")
                .field("username", username)
                .field("password", &RedactedSecret)
                .finish(),
        }
    }
}

impl Sasl {
    /// Uses the SASL PLAIN mechanism.
    #[must_use]
    pub fn plain(username: impl Into<String>, password: impl Into<Secret>) -> Self {
        Self {
            mechanisms: vec![Mechanism::Plain {
                username: username.into(),
                password: password.into(),
            }],
            required: true,
        }
    }

    /// Uses the RFC 7628 SASL OAUTHBEARER mechanism.
    #[must_use]
    pub fn oauth_bearer(token: impl Into<Secret>) -> Self {
        Self {
            mechanisms: vec![Mechanism::OAuthBearer {
                token: token.into(),
            }],
            required: true,
        }
    }

    /// Uses the SASL EXTERNAL mechanism with an empty authorization identity.
    #[must_use]
    pub fn external() -> Self {
        Self {
            mechanisms: vec![Mechanism::External { authzid: None }],
            required: true,
        }
    }

    /// Sets the authorization identity sent by SASL EXTERNAL.
    ///
    /// This has no effect on PLAIN or SCRAM-SHA-256 mechanisms.
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
    pub fn scram_sha256(username: impl Into<String>, password: impl Into<Secret>) -> Self {
        Self {
            mechanisms: vec![Mechanism::ScramSha256 {
                username: username.into(),
                password: password.into(),
            }],
            required: true,
        }
    }

    /// Prefers SCRAM-SHA-256 and falls back to PLAIN.
    #[must_use]
    pub fn any(username: impl Into<String>, password: impl Into<Secret>) -> Self {
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
            required: true,
        }
    }

    /// Allows registration to continue when every configured mechanism fails.
    ///
    /// ```
    /// let sasl = papo_core::Sasl::any("alice", "secret").optional();
    /// # _ = sasl;
    /// ```
    #[must_use]
    pub const fn optional(mut self) -> Self {
        self.required = false;
        self
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
    deadline: Option<Instant>,
    aborting: bool,
    scram: Option<ScramClient>,
    scram_step: ScramStep,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Completion {
    Pending,
    Success,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ScramStep {
    #[default]
    None,
    Initial,
    ServerFirst,
    ServerFinal,
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
            .field("deadline", &self.deadline)
            .field("aborting", &self.aborting)
            .field("has_scram", &self.scram.is_some())
            .field("scram_step", &self.scram_step)
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
            require: config.sasl.as_ref().is_some_and(|sasl| sasl.required),
            failures: Vec::new(),
            deadline: None,
            aborting: false,
            scram: None,
            scram_step: ScramStep::None,
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
        self.deadline = None;
        self.aborting = false;
        self.scram = None;
        self.scram_step = ScramStep::None;
    }

    pub(super) fn handle_cap_ack(
        &mut self,
        now: Instant,
        advertised: Option<&str>,
        outbox: &mut Outbox,
    ) {
        if self.completion != Completion::Pending && self.account.is_some() {
            return;
        }
        if self.completion != Completion::Pending {
            self.reset_exchange();
        }
        if let Some(advertised) = advertised {
            self.retain_available(advertised);
        }
        self.start_next(now, outbox);
    }

    pub(super) fn cap_removed(&mut self, outbox: &mut Outbox) {
        if self.completion == Completion::Pending {
            self.fail_all(
                "the sasl capability disappeared during authentication",
                outbox,
            );
        }
    }

    pub(super) fn reauthenticate(
        &mut self,
        now: Instant,
        available: bool,
        advertised: Option<&str>,
        outbox: &mut Outbox,
    ) {
        if self.configured.is_empty() || self.completion == Completion::Pending {
            return;
        }
        self.reset_exchange();
        if !available {
            self.fail_all("the server does not currently advertise sasl", outbox);
            return;
        }
        if let Some(advertised) = advertised {
            self.retain_available(advertised);
        }
        self.start_next(now, outbox);
    }

    fn reset_exchange(&mut self) {
        self.remaining = self.configured.clone().into();
        self.active = None;
        self.awaiting_challenge = false;
        self.completion = Completion::Pending;
        self.challenge.clear();
        self.failures.clear();
        self.deadline = None;
        self.aborting = false;
        self.scram = None;
        self.scram_step = ScramStep::None;
    }

    pub(super) fn handle_message(
        &mut self,
        now: Instant,
        message: &MessageRef<'_>,
        outbox: &mut Outbox,
    ) -> Handled {
        if self.handle_status(now, message, outbox) {
            return Handled::Yes;
        }
        if !message.command_is("AUTHENTICATE") {
            return Handled::No;
        }
        let Some(mechanism) = self.active.as_ref() else {
            return Handled::No;
        };
        if !self.awaiting_challenge {
            return Handled::No;
        }
        let Some(chunk) = message.param(0) else {
            self.abort(now, "the server omitted the SASL challenge payload", outbox);
            return Handled::Yes;
        };
        let challenge = match receive_challenge(&mut self.challenge, chunk) {
            Ok(Some(challenge)) => challenge,
            Ok(None) => {
                self.deadline = Some(now + SASL_TIMEOUT);
                return Handled::Yes;
            }
            Err(()) => {
                self.abort(now, "the server sent an invalid SASL challenge", outbox);
                return Handled::Yes;
            }
        };
        if matches!(mechanism, Mechanism::ScramSha256 { .. }) {
            match self.handle_scram_challenge(&challenge, outbox) {
                Ok(done) => self.awaiting_challenge = !done,
                Err(reason) => {
                    self.abort(now, &reason, outbox);
                    return Handled::Yes;
                }
            }
            self.deadline = Some(now + SASL_TIMEOUT);
            return Handled::Yes;
        }
        match mechanism {
            Mechanism::Plain { username, password } => {
                let authcid = with_device(username, self.device.as_deref());
                transmit_response(
                    format!("\0{authcid}\0{}", password.expose()).as_bytes(),
                    outbox,
                );
            }
            Mechanism::OAuthBearer { token } => {
                transmit_response(
                    format!("n,,\u{1}auth=Bearer {}\u{1}\u{1}", token.expose()).as_bytes(),
                    outbox,
                );
            }
            Mechanism::External { authzid } => {
                transmit_response(authzid.as_deref().unwrap_or("").as_bytes(), outbox);
            }
            Mechanism::ScramSha256 { .. } => unreachable!(),
        }
        self.awaiting_challenge = false;
        self.deadline = Some(now + SASL_TIMEOUT);
        Handled::Yes
    }

    fn handle_status(
        &mut self,
        now: Instant,
        message: &MessageRef<'_>,
        outbox: &mut Outbox,
    ) -> bool {
        if message.command_is("900") {
            if let Some(account) = message.param(2) {
                self.account = Some(account.to_owned());
            }
        } else if message.command_is("901") {
            self.account = None;
        } else if message.command_is("903")
            && matches!(self.active, Some(Mechanism::ScramSha256 { .. }))
            && self.scram_step != ScramStep::None
        {
            self.abort(
                now,
                "SCRAM succeeded without a verified server proof",
                outbox,
            );
            if self.require {
                self.completion = Completion::Failed;
                self.aborting = false;
                self.deadline = None;
                outbox.close(
                    crate::CloseKind::SaslFailed,
                    format!("SASL failed: {}", self.failures.join("; ")),
                );
            }
        } else if message.command_is("903") || message.command_is("907") {
            self.active = None;
            self.awaiting_challenge = false;
            self.deadline = None;
            self.scram = None;
            self.scram_step = ScramStep::None;
            self.completion = Completion::Success;
        } else if message.command_is("904") || message.command_is("905") {
            self.record_active_failure(message.param(1).unwrap_or("SASL mechanism failed"));
            self.start_next(now, outbox);
        } else if message.command_is("908") {
            if let Some(available) = message.param(1) {
                self.retain_available(available);
            }
        } else if message.command_is("902") {
            self.fail_all(message.param(1).unwrap_or("account is unavailable"), outbox);
        } else if message.command_is("906") {
            if self.aborting {
                self.aborting = false;
                self.start_next(now, outbox);
            } else {
                self.fail_all(
                    message.param(1).unwrap_or("SASL exchange was aborted"),
                    outbox,
                );
            }
        } else if message.command_is("FAIL")
            && message.param(0) == Some("*")
            && message.param(1) == Some("ACCOUNT_REQUIRED")
        {
            self.completion = Completion::Failed;
            self.deadline = None;
            outbox.close(
                crate::CloseKind::AccountRequired,
                format!(
                    "SASL failed: {}",
                    message.param(2).unwrap_or("the server requires an account")
                ),
            );
        } else {
            return false;
        }
        true
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

    pub(super) const fn next_deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub(super) fn handle_timeout(&mut self, now: Instant, outbox: &mut Outbox) {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.deadline = None;
            if self.aborting {
                self.fail_all("the server did not acknowledge the SASL abort", outbox);
            } else if self.active.is_some() {
                self.abort(now, "SASL mechanism timed out after 30 seconds", outbox);
            }
        }
    }

    pub(super) fn handle_disconnected(&mut self) {
        self.deadline = None;
    }

    fn start_next(&mut self, now: Instant, outbox: &mut Outbox) {
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
            if let Mechanism::ScramSha256 { username, password } = &mechanism {
                let mut nonce = [0; 18];
                self.entropy
                    .as_mut()
                    .expect("SCRAM entropy was checked")
                    .fill(&mut nonce);
                let authcid = with_device(username, self.device.as_deref());
                match ScramClient::new(&authcid, password.expose(), &STANDARD.encode(nonce)) {
                    Ok(scram) => {
                        self.scram = Some(scram);
                        self.scram_step = ScramStep::Initial;
                    }
                    Err(error) => {
                        self.failures.push(format!("SCRAM-SHA-256: {error:?}"));
                        continue;
                    }
                }
            }
            outbox.transmit("AUTHENTICATE", [mechanism.name()]);
            self.active = Some(mechanism);
            self.awaiting_challenge = true;
            self.challenge.clear();
            self.deadline = Some(now + SASL_TIMEOUT);
            return;
        }
    }

    fn record_active_failure(&mut self, reason: &str) {
        if let Some(mechanism) = self.active.take() {
            self.failures
                .push(format!("{}: {reason}", mechanism.name()));
        }
        self.awaiting_challenge = false;
        self.deadline = None;
        self.scram = None;
        self.scram_step = ScramStep::None;
    }

    fn retain_available(&mut self, available: &str) {
        self.remaining.retain(|mechanism| {
            available
                .split(',')
                .any(|name| name.eq_ignore_ascii_case(mechanism.name()))
        });
    }

    fn abort(&mut self, now: Instant, reason: &str, outbox: &mut Outbox) {
        self.record_active_failure(reason);
        self.aborting = true;
        self.deadline = Some(now + SASL_TIMEOUT);
        outbox.transmit("AUTHENTICATE", ["*"]);
    }

    fn handle_scram_challenge(
        &mut self,
        challenge: &[u8],
        outbox: &mut Outbox,
    ) -> Result<bool, String> {
        let client = self.scram.as_mut().ok_or("SCRAM client is missing")?;
        match self.scram_step {
            ScramStep::Initial if challenge.is_empty() => {
                transmit_response(client.client_first().as_bytes(), outbox);
                self.scram_step = ScramStep::ServerFirst;
                Ok(false)
            }
            ScramStep::ServerFirst => {
                let challenge = std::str::from_utf8(challenge)
                    .map_err(|_| "SCRAM server-first message is not UTF-8")?;
                let response = client
                    .handle_server_first(challenge)
                    .map_err(|error| format!("SCRAM server-first failed: {error:?}"))?;
                transmit_response(response.as_bytes(), outbox);
                self.scram_step = ScramStep::ServerFinal;
                Ok(false)
            }
            ScramStep::ServerFinal => {
                let challenge = std::str::from_utf8(challenge)
                    .map_err(|_| "SCRAM server-final message is not UTF-8")?;
                client
                    .handle_server_final(challenge)
                    .map_err(|error| format!("SCRAM server-final failed: {error:?}"))?;
                transmit_response(&[], outbox);
                self.scram_step = ScramStep::None;
                Ok(true)
            }
            _ => Err("unexpected SCRAM challenge".to_owned()),
        }
    }

    fn fail_all(&mut self, reason: &str, outbox: &mut Outbox) {
        self.record_active_failure(reason);
        self.completion = Completion::Failed;
        self.aborting = false;
        self.failures.push(reason.to_owned());
        let reason = format!("SASL failed: {}", self.failures.join("; "));
        if self.require {
            outbox.close(crate::CloseKind::SaslFailed, reason);
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
        if chunk.len() > SASL_CHUNK_LEN
            || buffer.len().saturating_add(chunk.len()) > MAX_SASL_CHALLENGE_LEN
        {
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
            Self::OAuthBearer { .. } => "OAUTHBEARER",
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
        let first = "YWFh".repeat(100);
        assert_eq!(first.len(), 400);
        assert_eq!(receive_challenge(&mut buffer, &first), Ok(None));
        assert_eq!(
            receive_challenge(&mut buffer, "+").unwrap().unwrap().len(),
            300
        );

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
        assert!(receive_challenge(&mut String::new(), "not+base64!").is_err());
    }

    #[test]
    fn accepts_a_literal_4096_byte_reassembled_challenge() {
        let mut buffer = String::new();
        for _ in 0..10 {
            assert_eq!(
                receive_challenge(&mut buffer, &"YWFh".repeat(100)),
                Ok(None)
            );
        }
        assert_eq!(
            receive_challenge(&mut buffer, &"YWFh".repeat(24))
                .unwrap()
                .unwrap()
                .len(),
            3072
        );
    }
}
