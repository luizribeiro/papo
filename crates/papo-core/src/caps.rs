use std::collections::BTreeMap;

use papo_proto::MessageRef;

use crate::{Config, Handled, Outbox};

pub(super) type Capabilities = BTreeMap<String, Option<String>>;

#[derive(Debug, Default)]
pub(super) struct Caps {
    advertised: Capabilities,
    enabled: Capabilities,
    ended: bool,
}

impl Caps {
    pub(super) fn handle_connected(&mut self) {
        *self = Self::default();
    }

    pub(super) fn handle_message(
        &mut self,
        message: &MessageRef<'_>,
        _config: &Config,
        authentication_finished: bool,
        outbox: &mut Outbox,
    ) -> Handled {
        if message.command_is("CAP") {
            let Some(subcommand) = message.param(1) else {
                return Handled::No;
            };
            if subcommand.eq_ignore_ascii_case("LS") {
                self.handle_ls(message, authentication_finished, outbox);
            } else {
                return Handled::No;
            }
            Handled::Yes
        } else if message.command_is("410")
            || (message.command_is("421")
                && message
                    .param(1)
                    .is_some_and(|command| command.eq_ignore_ascii_case("CAP")))
        {
            Handled::Yes
        } else {
            Handled::No
        }
    }

    fn handle_ls(
        &mut self,
        message: &MessageRef<'_>,
        authentication_finished: bool,
        outbox: &mut Outbox,
    ) {
        let continued = message.param(2) == Some("*");
        let list = message.param(if continued { 3 } else { 2 }).unwrap_or("");
        for token in list.split_whitespace() {
            let (name, value) = token
                .split_once('=')
                .map_or((token, None), |(name, value)| (name, Some(value)));
            if !name.is_empty() {
                self.advertised
                    .insert(name.to_owned(), value.map(str::to_owned));
            }
        }
        if continued {
            return;
        }

        self.enabled.insert("cap-notify".to_owned(), None);
        self.finish(authentication_finished, outbox);
    }

    fn finish(&mut self, authentication_finished: bool, outbox: &mut Outbox) {
        if authentication_finished && !self.ended {
            outbox.transmit_raw(b"CAP END\r\n".to_vec());
            self.ended = true;
        }
    }

    pub(super) const fn enabled(&self) -> &Capabilities {
        &self.enabled
    }
}
