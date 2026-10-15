use papo_proto::{Isupport, MessageRef};

use crate::{Decoding, Handled};

#[derive(Debug)]
pub(super) struct IsupportState {
    configured_decoding: Decoding,
    decoding: Decoding,
    value: Isupport,
}

impl IsupportState {
    pub(super) fn new(configured_decoding: Decoding) -> Self {
        Self {
            configured_decoding,
            decoding: configured_decoding,
            value: Isupport::default(),
        }
    }

    pub(super) fn handle_connected(&mut self) {
        self.decoding = self.configured_decoding;
        self.value = Isupport::default();
    }

    pub(super) fn handle_message(&mut self, message: &MessageRef<'_>) -> Handled {
        if !message.command_is("005") {
            return Handled::No;
        }
        self.value.apply(message);
        self.decoding = if self.value.utf8only() {
            Decoding::Utf8
        } else {
            self.configured_decoding
        };
        Handled::Yes
    }

    pub(super) const fn decoding(&self) -> Decoding {
        self.decoding
    }

    pub(super) const fn value(&self) -> &Isupport {
        &self.value
    }
}
