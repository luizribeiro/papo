pub(crate) mod scram;

use std::{collections::VecDeque, time::Duration};
use web_time::Instant;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use papo_proto::LineRef;
use zeroize::Zeroizing;

use crate::{Config, Handled, Outbox, Secret, sts::ConnectionSecurity};
use scram::ScramClient;

const SASL_CHUNK_LEN: usize = 400;
const MAX_SASL_CHALLENGE_LEN: usize = 4096;
const SASL_TIMEOUT: Duration = Duration::from_secs(30);

/// Ordered SASL mechanisms to try during connection registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sasl {
    pub(crate) mechanisms: Vec<Mechanism>,
    pub(crate) required: bool,
    device: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Mechanism {
    Plain {
        username: String,
        password: Secret,
        authzid: Option<String>,
    },
    OAuthBearer {
        token: Secret,
    },
    External {
        authzid: Option<String>,
    },
    ScramSha256 {
        username: String,
        password: Secret,
    },
}

impl Sasl {
    /// Reports whether a configured credential contains bytes rather than text.
    #[doc(hidden)]
    #[must_use]
    pub fn has_binary_secret(&self) -> bool {
        self.mechanisms.iter().any(|mechanism| match mechanism {
            Mechanism::Plain { password, .. } | Mechanism::ScramSha256 { password, .. } => {
                password.expose().is_none()
            }
            Mechanism::OAuthBearer { token } => token.expose().is_none(),
            Mechanism::External { .. } => false,
        })
    }

    /// Uses the SASL PLAIN mechanism.
    #[must_use]
    pub fn plain(username: impl Into<String>, password: impl Into<Secret>) -> Self {
        Self {
            mechanisms: vec![Mechanism::Plain {
                username: username.into(),
                password: password.into(),
                authzid: None,
            }],
            required: true,
            device: None,
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
            device: None,
        }
    }

    /// Uses the SASL EXTERNAL mechanism with an empty authorization identity.
    #[must_use]
    pub fn external() -> Self {
        Self {
            mechanisms: vec![Mechanism::External { authzid: None }],
            required: true,
            device: None,
        }
    }

    /// Sets the authorization identity sent by SASL PLAIN or EXTERNAL.
    ///
    /// This has no effect on SCRAM-SHA-256 or OAUTHBEARER mechanisms.
    #[must_use]
    pub fn authzid(mut self, authzid: impl Into<String>) -> Self {
        for mechanism in &mut self.mechanisms {
            match mechanism {
                Mechanism::Plain {
                    authzid: current, ..
                }
                | Mechanism::External { authzid: current } => {
                    *current = Some(authzid.into());
                    break;
                }
                Mechanism::OAuthBearer { .. } | Mechanism::ScramSha256 { .. } => {}
            }
        }
        self
    }

    /// Appends `@<device>` to the PLAIN or SCRAM-SHA-256 authentication identity.
    ///
    /// Bouncers and servers can use the suffix to distinguish a user's clients.
    #[must_use]
    pub fn device(mut self, device: impl Into<String>) -> Self {
        self.device = Some(device.into());
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
            device: None,
        }
    }

    /// Prefers SCRAM-SHA-256 and falls back to PLAIN.
    ///
    /// PLAIN fallback is allowed only on a secure connection and before the
    /// client answers any SCRAM challenge. Otherwise the SASL attempt fails.
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
                Mechanism::Plain {
                    username,
                    password,
                    authzid: None,
                },
            ],
            required: true,
            device: None,
        }
    }

    /// Allows connecting to continue when every configured mechanism fails.
    ///
    /// ```
    /// # use papo_core as papo;
    /// let sasl = papo::Sasl::any("alice", "secret").optional();
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
    failure_kind: FailureKind,
    deadline: Option<Instant>,
    aborting: bool,
    abort_outcome: AbortOutcome,
    plain_fallback: PlainFallback,
    scram: Option<ScramClient>,
    scram_step: ScramStep,
    proof: Proof,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Completion {
    Pending,
    Success,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FailureKind {
    #[default]
    Login,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ScramStep {
    #[default]
    None,
    Initial,
    ServerFirst,
    ServerFinal,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Proof {
    #[default]
    Unverified,
    Verified,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum AbortOutcome {
    #[default]
    Retry,
    Fail,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlainFallback {
    NotConfigured,
    Allowed,
    Forbidden,
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
            .field("failure_kind", &self.failure_kind)
            .field("deadline", &self.deadline)
            .field("aborting", &self.aborting)
            .field("abort_outcome", &self.abort_outcome)
            .field("plain_fallback", &self.plain_fallback)
            .field("has_scram", &self.scram.is_some())
            .field("scram_step", &self.scram_step)
            .field("proof", &self.proof)
            .finish()
    }
}

impl SaslSession {
    pub(super) fn new(config: &Config, entropy: Option<Box<dyn Entropy + Send>>) -> Self {
        let configured = config
            .sasl
            .as_ref()
            .map_or_else(Vec::new, |sasl| sasl.mechanisms.clone());
        let plain_fallback = Self::plain_fallback(&configured, config.connection_security);
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
            device: config.sasl.as_ref().and_then(|sasl| sasl.device.clone()),
            challenge: String::new(),
            entropy,
            require: config.sasl.as_ref().is_some_and(|sasl| sasl.required),
            failures: Vec::new(),
            failure_kind: FailureKind::Login,
            deadline: None,
            aborting: false,
            abort_outcome: AbortOutcome::Retry,
            plain_fallback,
            scram: None,
            scram_step: ScramStep::None,
            proof: Proof::Unverified,
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
        self.failure_kind = FailureKind::Login;
        self.deadline = None;
        self.aborting = false;
        self.abort_outcome = AbortOutcome::Retry;
        self.scram = None;
        self.scram_step = ScramStep::None;
        self.proof = Proof::Unverified;
    }

    pub(super) fn handle_cap_ack(
        &mut self,
        now: Instant,
        security: ConnectionSecurity,
        advertised: Option<&str>,
        outbox: &mut Outbox,
    ) {
        if self.completion != Completion::Pending && self.account.is_some() {
            return;
        }
        if self.completion != Completion::Pending {
            self.reset_exchange();
        }
        self.plain_fallback = Self::plain_fallback(&self.configured, security);
        if let Some(advertised) = advertised {
            self.retain_available(advertised);
        }
        self.start_next(now, outbox);
    }

    pub(super) fn cap_removed(&mut self, outbox: &mut Outbox) {
        if self.completion == Completion::Pending {
            self.failure_kind = FailureKind::Unavailable;
            self.fail_all(
                "the sasl capability disappeared during authentication",
                outbox,
            );
        }
    }

    pub(super) fn reauthenticate(
        &mut self,
        now: Instant,
        security: ConnectionSecurity,
        available: bool,
        advertised: Option<&str>,
        outbox: &mut Outbox,
    ) {
        if self.configured.is_empty() || self.completion == Completion::Pending {
            return;
        }
        self.reset_exchange();
        self.plain_fallback = Self::plain_fallback(&self.configured, security);
        if !available {
            self.failure_kind = FailureKind::Unavailable;
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
        self.failure_kind = FailureKind::Login;
        self.deadline = None;
        self.aborting = false;
        self.abort_outcome = AbortOutcome::Retry;
        self.scram = None;
        self.scram_step = ScramStep::None;
        self.proof = Proof::Unverified;
    }

    pub(super) fn handle_message(
        &mut self,
        now: Instant,
        message: &LineRef<'_>,
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
            self.abort(
                now,
                "the server omitted the SASL challenge payload",
                AbortOutcome::Retry,
                outbox,
            );
            return Handled::Yes;
        };
        let challenge = match receive_challenge(&mut self.challenge, chunk) {
            Ok(Some(challenge)) => challenge,
            Ok(None) => {
                self.deadline = Some(now + SASL_TIMEOUT);
                return Handled::Yes;
            }
            Err(()) => {
                self.abort(
                    now,
                    "the server sent an invalid SASL challenge",
                    AbortOutcome::Retry,
                    outbox,
                );
                return Handled::Yes;
            }
        };
        if matches!(mechanism, Mechanism::ScramSha256 { .. }) {
            match self.handle_scram_challenge(&challenge, outbox) {
                Ok(done) => self.awaiting_challenge = !done,
                Err(reason) => {
                    self.abort(now, &reason, AbortOutcome::Fail, outbox);
                    return Handled::Yes;
                }
            }
            self.deadline = Some(now + SASL_TIMEOUT);
            return Handled::Yes;
        }
        let response = match mechanism {
            Mechanism::Plain {
                username,
                password,
                authzid,
            } => {
                let authcid = with_device(username, self.device.as_deref());
                password.expose().map(|password| {
                    let authorization_identity = authzid.as_deref().unwrap_or("");
                    let mut response = Zeroizing::new(Vec::with_capacity(
                        authorization_identity.len() + authcid.len() + password.len() + 2,
                    ));
                    response.extend_from_slice(authorization_identity.as_bytes());
                    response.push(0);
                    response.extend_from_slice(authcid.as_bytes());
                    response.push(0);
                    response.extend_from_slice(password.as_bytes());
                    response
                })
            }
            Mechanism::OAuthBearer { token } => token.expose().map(|token| {
                let mut response = Zeroizing::new(Vec::with_capacity(token.len() + 18));
                response.extend_from_slice(b"n,,\x01auth=Bearer ");
                response.extend_from_slice(token.as_bytes());
                response.extend_from_slice(b"\x01\x01");
                response
            }),
            Mechanism::External { authzid } => Some(Zeroizing::new(
                authzid.as_deref().unwrap_or("").as_bytes().to_vec(),
            )),
            // SCRAM challenges return through `handle_scram_challenge` above.
            Mechanism::ScramSha256 { .. } => unreachable!(),
        };
        let Some(response) = response else {
            self.abort(
                now,
                "the configured SASL secret is not text",
                AbortOutcome::Retry,
                outbox,
            );
            return Handled::Yes;
        };
        transmit_response(&response, outbox);
        self.awaiting_challenge = false;
        self.deadline = Some(now + SASL_TIMEOUT);
        Handled::Yes
    }

    fn handle_status(&mut self, now: Instant, message: &LineRef<'_>, outbox: &mut Outbox) -> bool {
        if message.command_is("900") {
            if let Some(account) = message.param(2) {
                self.account = Some(account.to_owned());
            }
        } else if message.command_is("901") {
            self.account = None;
        } else if (message.command_is("903") || message.command_is("907"))
            && (self.aborting
                || (matches!(self.active, Some(Mechanism::ScramSha256 { .. }))
                    && self.proof != Proof::Verified))
        {
            self.fail_all("SCRAM succeeded without a verified server proof", outbox);
        } else if message.command_is("903") || message.command_is("907") {
            if self.active.is_some() {
                self.active = None;
                self.awaiting_challenge = false;
                self.deadline = None;
                self.scram = None;
                self.scram_step = ScramStep::None;
                self.completion = Completion::Success;
            }
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
                if self.abort_outcome == AbortOutcome::Retry {
                    self.start_next(now, outbox);
                } else {
                    self.fail_all("SCRAM exchange failed", outbox);
                }
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
                message
                    .param(2)
                    .unwrap_or("the server requires an account")
                    .to_owned(),
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
        self.failure_kind = FailureKind::Unavailable;
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
                self.failure_kind = FailureKind::Unavailable;
                self.fail_all("the server did not acknowledge the SASL abort", outbox);
            } else if self.active.is_some() {
                self.failure_kind = FailureKind::Unavailable;
                self.abort(
                    now,
                    "SASL mechanism timed out after 30 seconds",
                    AbortOutcome::Retry,
                    outbox,
                );
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
            if matches!(mechanism, Mechanism::Plain { .. })
                && self.plain_fallback == PlainFallback::Forbidden
            {
                self.failures.push(
                    "PLAIN fallback is allowed only on a secure connection before a SCRAM challenge"
                        .to_owned(),
                );
                continue;
            }
            if matches!(mechanism, Mechanism::ScramSha256 { .. }) && self.entropy.is_none() {
                self.failures.push(
                    "SCRAM-SHA-256 skipped because no entropy source was provided".to_owned(),
                );
                continue;
            }
            if let Mechanism::ScramSha256 { username, password } = &mechanism {
                if password.expose().is_none() {
                    self.failures
                        .push("SCRAM-SHA-256: configured secret is not text".to_owned());
                    continue;
                }
                let mut nonce = [0; 18];
                let Some(entropy) = self.entropy.as_mut() else {
                    continue;
                };
                entropy.fill(&mut nonce);
                let authcid = with_device(username, self.device.as_deref());
                match ScramClient::new(&authcid, password, &STANDARD.encode(nonce)) {
                    Ok(scram) => {
                        self.scram = Some(scram);
                        self.scram_step = ScramStep::Initial;
                        self.proof = Proof::Unverified;
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

    fn abort(&mut self, now: Instant, reason: &str, outcome: AbortOutcome, outbox: &mut Outbox) {
        self.record_active_failure(reason);
        self.aborting = true;
        self.abort_outcome = outcome;
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
                if self.plain_fallback == PlainFallback::Allowed {
                    self.plain_fallback = PlainFallback::Forbidden;
                }
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
                self.proof = Proof::Verified;
                Ok(true)
            }
            _ => Err("unexpected SCRAM challenge".to_owned()),
        }
    }

    fn fail_all(&mut self, reason: &str, outbox: &mut Outbox) {
        self.record_active_failure(reason);
        self.completion = Completion::Failed;
        self.aborting = false;
        self.abort_outcome = AbortOutcome::Retry;
        self.failures.push(reason.to_owned());
        let reason = self.failures.join("; ");
        if self.require {
            outbox.close(
                if self.failure_kind == FailureKind::Unavailable {
                    crate::CloseKind::SaslUnavailable
                } else {
                    crate::CloseKind::SaslFailed
                },
                reason,
            );
        } else {
            outbox.event(crate::Event::AuthenticationFailed { reason });
        }
    }

    fn plain_fallback(configured: &[Mechanism], security: ConnectionSecurity) -> PlainFallback {
        if !matches!(
            configured,
            [Mechanism::ScramSha256 { .. }, Mechanism::Plain { .. }]
        ) {
            PlainFallback::NotConfigured
        } else if security == ConnectionSecurity::Secure {
            PlainFallback::Allowed
        } else {
            PlainFallback::Forbidden
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
    let capacity = base64::encoded_len(response.len(), true).unwrap_or(usize::MAX);
    let mut encoded = Zeroizing::new(String::with_capacity(capacity));
    STANDARD.encode_string(response, &mut encoded);
    if encoded.is_empty() {
        outbox.transmit("AUTHENTICATE", ["+"]);
        return;
    }
    for chunk in encoded.as_bytes().chunks(SASL_CHUNK_LEN) {
        let chunk = std::str::from_utf8(chunk).expect("base64 is ASCII");
        match papo_proto::Line::new("AUTHENTICATE")
            .param(Secret::new(chunk))
            .encode()
        {
            Ok(line) => outbox.transmit_raw(line),
            Err(error) => outbox.close(
                crate::CloseKind::Other,
                format!("invalid SASL response: {error}"),
            ),
        }
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
