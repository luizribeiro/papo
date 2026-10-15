use std::collections::{BTreeMap, BTreeSet};

use papo_proto::{MessageRef, NAME_ALIASES, NameKind};

use crate::{Config, Handled, Outbox};

pub(super) type Capabilities = BTreeMap<String, Option<String>>;

#[derive(Debug, Default)]
pub(super) struct Caps {
    advertised: Capabilities,
    enabled: Capabilities,
    pending: Option<Vec<String>>,
    ended: bool,
}

impl Caps {
    pub(super) fn handle_connected(&mut self) {
        *self = Self::default();
    }

    pub(super) fn handle_message(
        &mut self,
        message: &MessageRef<'_>,
        config: &Config,
        authentication_finished: bool,
        outbox: &mut Outbox,
    ) -> Handled {
        if message.command_is("CAP") {
            let Some(subcommand) = message.param(1) else {
                return Handled::No;
            };
            if subcommand.eq_ignore_ascii_case("LS") {
                self.handle_ls(message, config, authentication_finished, outbox);
            } else if subcommand.eq_ignore_ascii_case("ACK") {
                if let Some(requested) = self.pending.take() {
                    for name in requested {
                        let value = self.advertised.get(&name).cloned().flatten();
                        self.enabled.insert(name, value);
                    }
                }
                self.finish(authentication_finished, outbox);
            } else if subcommand.eq_ignore_ascii_case("NAK") {
                self.pending = None;
                self.finish(authentication_finished, outbox);
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
        config: &Config,
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
        let mut seen = BTreeSet::new();
        let requested = config
            .wanted_caps
            .iter()
            // Ergo disconnects pre-registration clients that request this trap capability.
            .filter(|wanted| wanted.as_str() != "ergo.chat/nope")
            .filter_map(|wanted| self.advertised_spelling(wanted))
            .filter(|name| seen.insert((*name).to_owned()))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if requested.is_empty() {
            self.finish(authentication_finished, outbox);
        } else {
            outbox.transmit("CAP", ["REQ", &requested.join(" ")]);
            self.pending = Some(requested);
        }
    }

    fn advertised_spelling(&self, wanted: &str) -> Option<&str> {
        if let Some((name, _)) = self.advertised.get_key_value(wanted) {
            return Some(name);
        }
        NAME_ALIASES.iter().find_map(|&(kind, draft, ratified)| {
            (kind == NameKind::Capability && (wanted == draft || wanted == ratified))
                .then(|| {
                    [draft, ratified].into_iter().find_map(|alias| {
                        self.advertised.get_key_value(alias).map(|(name, _)| name)
                    })
                })
                .flatten()
                .map(String::as_str)
        })
    }

    fn finish(&mut self, authentication_finished: bool, outbox: &mut Outbox) {
        if authentication_finished && self.pending.is_none() && !self.ended {
            outbox.transmit_raw(b"CAP END\r\n".to_vec());
            self.ended = true;
        }
    }

    pub(super) const fn enabled(&self) -> &Capabilities {
        &self.enabled
    }
}
