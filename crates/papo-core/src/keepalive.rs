use std::time::Duration;
use web_time::Instant;

use papo_proto::LineRef;

use crate::{CloseKind, Handled, Outbox};

const MAX_SERVER_ERROR_REASON_LEN: usize = 256;

#[derive(Debug, Default)]
pub(super) struct Keepalive {
    interval: Duration,
    timeout: Duration,
    next_ping: Option<Instant>,
    silence_deadline: Option<Instant>,
    pending_ping: Option<(u64, Instant)>,
    counter: u64,
    lag: Option<Duration>,
}

impl Keepalive {
    pub(super) fn handle_connected(&mut self, now: Instant, interval: Duration, timeout: Duration) {
        self.interval = interval;
        self.timeout = timeout;
        self.next_ping = Some(now + interval);
        self.silence_deadline = Some(now + timeout);
        self.pending_ping = None;
        self.counter = 0;
        self.lag = None;
    }

    pub(super) fn handle_incoming(&mut self, now: Instant) {
        self.next_ping = Some(now + self.interval);
        self.silence_deadline = Some(now + self.timeout);
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
            let mut pong = Vec::with_capacity(token.len() + 8);
            pong.extend_from_slice(b"PONG :");
            pong.extend_from_slice(token.as_bytes());
            pong.extend_from_slice(b"\r\n");
            outbox.transmit_raw(pong);
            Handled::Yes
        } else if message.command_is("PONG") {
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
        if self
            .silence_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            outbox.close(
                CloseKind::PingTimeout,
                "IRC connection timed out".to_owned(),
            );
            return;
        }
        if self.next_ping.is_some_and(|deadline| now >= deadline) {
            let token = format!("papo-{}", self.counter);
            outbox.transmit_raw(format!("PING :{token}\r\n").into_bytes());
            self.pending_ping = Some((self.counter, now));
            self.counter = self.counter.wrapping_add(1);
            self.next_ping = None;
        }
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        match (self.next_ping, self.silence_deadline) {
            (Some(ping), Some(silence)) => Some(if ping < silence { ping } else { silence }),
            (ping, silence) => ping.or(silence),
        }
    }

    pub(super) const fn lag(&self) -> Option<Duration> {
        self.lag
    }

    pub(super) fn handle_disconnected(&mut self) {
        self.next_ping = None;
        self.silence_deadline = None;
        self.pending_ping = None;
    }
}

fn bounded_server_error_reason(reason: &str) -> String {
    let end = reason.floor_char_boundary(reason.len().min(MAX_SERVER_ERROR_REASON_LEN));
    reason[..end].to_owned()
}
