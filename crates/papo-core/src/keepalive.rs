use std::time::Instant;

use papo_proto::MessageRef;

use crate::{Handled, Outbox};

#[derive(Debug, Default)]
pub(super) struct Keepalive;

#[allow(clippy::unused_self)]
impl Keepalive {
    pub(super) fn handle_message(
        &mut self,
        _now: Instant,
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
        } else if message.command_is("ERROR") {
            outbox.close(message.param(0).unwrap_or("server sent ERROR").to_owned());
            Handled::Yes
        } else {
            Handled::No
        }
    }

    pub(super) fn handle_timeout(&mut self, _now: Instant, _outbox: &mut Outbox) {}

    pub(super) const fn next_deadline(&self) -> Option<Instant> {
        None
    }
}
