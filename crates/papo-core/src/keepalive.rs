use std::time::Duration;
use web_time::Instant;

use papo_proto::{Line, LineRef};

use crate::{CloseKind, Handled, Outbox};

const MAX_SERVER_ERROR_REASON_LEN: usize = 256;
const PING_TIMEOUT_REASON: &str = "IRC connection timed out";

/// Outcome of the active connection check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionCheckResult {
    /// A matching PONG arrived before the deadline.
    Pong(Duration),
    /// Other traffic arrived but the expected PONG did not.
    TimedOut,
    /// Silence through the deadline caused the connection to close.
    Silent,
}

#[derive(Debug)]
struct ConnectionCheck {
    token: String,
    sent_at: Instant,
    deadline: Option<Instant>,
    heard_traffic: bool,
}

#[derive(Debug, Default)]
pub(super) struct Keepalive {
    interval: Duration,
    timeout: Duration,
    next_ping: Option<Instant>,
    silence_deadline: Option<Instant>,
    pending_ping: Option<(u64, Instant)>,
    counter: u64,
    lag: Option<Duration>,
    check: Option<ConnectionCheck>,
    pub(super) check_result: Option<ConnectionCheckResult>,
}

impl Keepalive {
    pub(super) fn handle_connected(&mut self, now: Instant, interval: Duration, timeout: Duration) {
        self.interval = interval;
        self.timeout = timeout;
        self.next_ping = crate::deadline(now, interval);
        self.silence_deadline = crate::deadline(now, timeout);
        self.pending_ping = None;
        self.counter = 0;
        self.lag = None;
        self.check = None;
        self.check_result = None;
    }

    pub(super) fn start_check(&mut self, now: Instant, duration: Duration, outbox: &mut Outbox) {
        if self.check.is_some() {
            return;
        }
        let token = format!("papo-check-{}", self.counter);
        self.counter = self.counter.wrapping_add(1);
        outbox.transmit_immediate("PING", [token.as_str()]);
        self.check = Some(ConnectionCheck {
            token,
            sent_at: now,
            deadline: crate::deadline(now, duration),
            heard_traffic: false,
        });
    }

    fn expire_check(&mut self, now: Instant, outbox: &mut Outbox) -> bool {
        let Some(check) = &self.check else {
            return false;
        };
        if check.deadline.is_none_or(|deadline| now < deadline) {
            return false;
        }
        let silent = !check.heard_traffic;
        self.check = None;
        self.check_result = Some(if silent {
            outbox.close(CloseKind::PingTimeout, PING_TIMEOUT_REASON.to_owned());
            outbox.abort_transmits();
            ConnectionCheckResult::Silent
        } else {
            ConnectionCheckResult::TimedOut
        });
        silent
    }

    pub(super) fn handle_incoming(&mut self, now: Instant, outbox: &mut Outbox) -> bool {
        if self.expire_check(now, outbox) {
            return false;
        }
        if let Some(check) = &mut self.check {
            check.heard_traffic = true;
        }
        self.next_ping = crate::deadline(now, self.interval);
        self.silence_deadline = crate::deadline(now, self.timeout);
        true
    }

    pub(super) fn handle_message(
        &mut self,
        now: Instant,
        message: &LineRef<'_>,
        outbox: &mut Outbox,
    ) -> Handled {
        if message.command_is("PING") {
            let Some(token) = message.param(0) else {
                return Handled::No;
            };
            if let Ok(pong) = Line::new("PONG").param(token).encode() {
                outbox.transmit_raw(pong);
            }
            Handled::Yes
        } else if message.command_is("PONG") {
            let token = message.params().last();
            if let Some(check) = &self.check
                && token == Some(check.token.as_str())
            {
                self.check_result = Some(ConnectionCheckResult::Pong(
                    now.saturating_duration_since(check.sent_at),
                ));
                self.check = None;
            }
            if let Some((counter, sent_at)) = self.pending_ping {
                let token = format!("papo-{counter}");
                if message.params().last() == Some(token.as_str()) {
                    self.lag = Some(now.saturating_duration_since(sent_at));
                    self.pending_ping = None;
                }
            }
            Handled::Yes
        } else if message.command_is("ERROR") {
            outbox.close(
                CloseKind::ServerError,
                bounded_server_error_reason(message.param(0).unwrap_or("server sent ERROR")),
            );
            Handled::Yes
        } else {
            Handled::No
        }
    }

    pub(super) fn handle_timeout(&mut self, now: Instant, outbox: &mut Outbox) {
        if self.expire_check(now, outbox) {
            return;
        }
        if self
            .silence_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            outbox.close(CloseKind::PingTimeout, PING_TIMEOUT_REASON.to_owned());
            return;
        }
        if self.next_ping.is_some_and(|deadline| now >= deadline) {
            let token = format!("papo-{}", self.counter);
            if let Ok(ping) = Line::new("PING").param(token).encode() {
                outbox.transmit_raw(ping);
            }
            self.pending_ping = Some((self.counter, now));
            self.counter = self.counter.wrapping_add(1);
            self.next_ping = None;
        }
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        [
            self.next_ping,
            self.silence_deadline,
            self.check.as_ref().and_then(|check| check.deadline),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub(super) const fn lag(&self) -> Option<Duration> {
        self.lag
    }

    pub(super) fn handle_disconnected(&mut self) {
        self.next_ping = None;
        self.silence_deadline = None;
        self.pending_ping = None;
        self.check = None;
    }
}

fn bounded_server_error_reason(reason: &str) -> String {
    let end = reason.floor_char_boundary(reason.len().min(MAX_SERVER_ERROR_REASON_LEN));
    reason[..end].to_owned()
}
