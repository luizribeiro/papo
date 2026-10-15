use std::time::{Duration, Instant};

use papo_proto::MessageRef;

use crate::{Handled, Outbox};

#[derive(Debug, Default)]
pub(super) struct Keepalive {
    interval: Duration,
    next_ping: Option<Instant>,
    pending_ping: Option<(u64, Instant)>,
    counter: u64,
    lag: Option<Duration>,
}

impl Keepalive {
    pub(super) fn handle_connected(&mut self, now: Instant, interval: Duration) {
        self.interval = interval;
        self.next_ping = Some(now + interval);
        self.pending_ping = None;
        self.counter = 0;
        self.lag = None;
    }

    pub(super) fn handle_incoming(&mut self, now: Instant) {
        self.next_ping = Some(now + self.interval);
    }

    pub(super) fn handle_message(
        &mut self,
        now: Instant,
        message: &MessageRef<'_>,
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
            outbox.close(message.param(0).unwrap_or("server sent ERROR").to_owned());
            Handled::Yes
        } else {
            Handled::No
        }
    }

    pub(super) fn handle_timeout(&mut self, now: Instant, outbox: &mut Outbox) {
        if self.next_ping.is_some_and(|deadline| now >= deadline) {
            let token = format!("papo-{}", self.counter);
            outbox.transmit_raw(format!("PING :{token}\r\n").into_bytes());
            self.pending_ping = Some((self.counter, now));
            self.counter = self.counter.wrapping_add(1);
            self.next_ping = None;
        }
    }

    pub(super) const fn next_deadline(&self) -> Option<Instant> {
        self.next_ping
    }

    pub(super) const fn lag(&self) -> Option<Duration> {
        self.lag
    }

    pub(super) fn handle_disconnected(&mut self) {
        self.next_ping = None;
        self.pending_ping = None;
    }
}
