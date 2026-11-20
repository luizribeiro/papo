use papo_proto::{Casemapping, Isupport, Line, LineRef};

use crate::{Decoding, Event, Handled, Outbox, StateLimit};

#[derive(Debug)]
pub(super) struct IsupportState {
    configured_decoding: Decoding,
    decoding: Decoding,
    value: Isupport,
    limit_warned: bool,
}

fn seeded(casemapping: Casemapping) -> Isupport {
    let mut value = Isupport::default();
    value.__seed_casemapping(casemapping);
    value
}

impl IsupportState {
    pub(super) fn new(configured_decoding: Decoding, casemapping: Casemapping) -> Self {
        Self {
            configured_decoding,
            decoding: configured_decoding,
            value: seeded(casemapping),
            limit_warned: false,
        }
    }

    pub(super) fn handle_connected(&mut self, casemapping: Casemapping) {
        self.decoding = self.configured_decoding;
        self.value = seeded(casemapping);
        self.limit_warned = false;
    }

    pub(super) fn handle_message(&mut self, message: &LineRef<'_>, outbox: &mut Outbox) -> Handled {
        if !message.command_is("005") {
            return Handled::No;
        }
        self.value.apply(message);
        self.warn_limit(outbox);
        self.decoding = if self.value.utf8only() {
            Decoding::Utf8
        } else {
            self.configured_decoding
        };
        Handled::Yes
    }

    pub(super) fn handle_batch(&mut self, messages: &[Line], outbox: &mut Outbox) {
        let mut value = self.value.clone();
        for message in messages {
            value.apply(&message.as_ref());
        }
        self.value = value;
        self.warn_limit(outbox);
        self.decoding = if self.value.utf8only() {
            Decoding::Utf8
        } else {
            self.configured_decoding
        };
    }

    pub(super) const fn decoding(&self) -> Decoding {
        self.decoding
    }

    pub(super) const fn value(&self) -> &Isupport {
        &self.value
    }

    fn warn_limit(&mut self, outbox: &mut Outbox) {
        if self.value.__limit_reached() && !std::mem::replace(&mut self.limit_warned, true) {
            outbox.event(Event::StateLimitReached {
                limit: StateLimit::IsupportTokens,
            });
        }
    }
}
