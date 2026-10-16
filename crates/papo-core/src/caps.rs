use std::collections::{BTreeSet, VecDeque};
use web_time::Instant;

use papo_proto::{MAX_MESSAGE_LEN, MessageRef, NAME_ALIASES, NameKind};

use crate::sts::{StsAction, StsDirective, decide};
use crate::{
    Capabilities, CloseKind, Config, Connection, Event, Handled, Outbox, sasl::SaslSession,
};

const CAP_REQ_PREFIX_LEN: usize = b"CAP REQ :".len();
const MAX_IRC_BODY_LEN: usize = MAX_MESSAGE_LEN - 2;

#[derive(Debug, Default)]
pub(super) struct Caps {
    advertised: Capabilities,
    enabled: Capabilities,
    pending: VecDeque<Vec<String>>,
    negotiating: bool,
    ended: bool,
}

impl Caps {
    pub(super) fn handle_connected(&mut self, _now: Instant) {
        *self = Self::default();
    }

    pub(super) fn handle_message(
        &mut self,
        now: Instant,
        message: &MessageRef<'_>,
        config: &Config,
        connection: &Connection,
        sasl: &mut SaslSession,
        outbox: &mut Outbox,
    ) -> Handled {
        if message.command_is("CAP") {
            let Some(subcommand) = message.param(1) else {
                return Handled::No;
            };
            if subcommand.eq_ignore_ascii_case("LS") {
                self.handle_ls(message, config, connection, sasl, outbox);
            } else if subcommand.eq_ignore_ascii_case("NEW") {
                self.handle_new(message, config, connection, outbox);
            } else if subcommand.eq_ignore_ascii_case("DEL") {
                self.handle_del(message, sasl, outbox);
            } else if subcommand.eq_ignore_ascii_case("ACK") {
                if message.param(2) != Some("*")
                    && let Some(requested) = self.pending.pop_front()
                {
                    let sasl_acked = requested.iter().any(|name| name == "sasl");
                    let isupport_acked = requested
                        .iter()
                        .any(|name| name == "draft/extended-isupport");
                    let mut added = Capabilities::new();
                    for name in requested {
                        let value = self.advertised.get(&name).cloned().flatten();
                        self.enabled.insert(name.clone(), value.clone());
                        added.insert(name, value);
                    }
                    if sasl_acked {
                        sasl.handle_cap_ack(
                            now,
                            self.enabled.get("sasl").and_then(Option::as_deref),
                            outbox,
                        );
                    }
                    if isupport_acked {
                        outbox.transmit_immediate("ISUPPORT", []);
                    }
                    if self.ended {
                        outbox.event(Event::CapsChanged {
                            added,
                            removed: Capabilities::new(),
                        });
                    }
                }
            } else if subcommand.eq_ignore_ascii_case("NAK") {
                if self
                    .pending
                    .pop_front()
                    .is_some_and(|request| request.iter().any(|name| name == "sasl"))
                {
                    sasl.unavailable("the server rejected the sasl capability", outbox);
                }
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

    fn handle_del(
        &mut self,
        message: &MessageRef<'_>,
        sasl: &mut SaslSession,
        outbox: &mut Outbox,
    ) {
        let mut removed = Capabilities::new();
        let mut sasl_removed = false;
        for name in message.param(2).unwrap_or("").split_whitespace() {
            if name == "sts" {
                continue;
            }
            self.advertised.remove(name);
            if let Some(value) = self.enabled.remove(name) {
                removed.insert(name.to_owned(), value);
            }
            if name == "sasl" {
                sasl_removed = true;
            }
        }
        if !removed.is_empty() {
            outbox.event(Event::CapsChanged {
                added: Capabilities::new(),
                removed,
            });
        }
        if sasl_removed {
            sasl.cap_removed(outbox);
        }
    }

    fn handle_new(
        &mut self,
        message: &MessageRef<'_>,
        config: &Config,
        connection: &Connection,
        outbox: &mut Outbox,
    ) {
        let mut updated = Capabilities::new();
        let mut announced = BTreeSet::new();
        for token in message.param(2).unwrap_or("").split_whitespace() {
            let (name, value) = token
                .split_once('=')
                .map_or((token, None), |(name, value)| (name, Some(value)));
            if name.is_empty() {
                continue;
            }
            let value = value.map(str::to_owned);
            self.advertised.insert(name.to_owned(), value.clone());
            announced.insert(name.to_owned());
            if self.enabled.contains_key(name) {
                self.enabled.insert(name.to_owned(), value.clone());
                updated.insert(name.to_owned(), value);
            }
        }
        if announced.contains("sts") && self.handle_sts(connection, outbox) {
            return;
        }
        if !updated.is_empty() {
            outbox.event(Event::CapsChanged {
                added: updated,
                removed: Capabilities::new(),
            });
        }

        let mut seen = BTreeSet::new();
        let modern_metadata = config
            .wanted_caps
            .iter()
            .any(|name| matches!(name.as_str(), "draft/metadata-2" | "draft/metadata-3"))
            && ["draft/metadata-3", "draft/metadata-2"]
                .iter()
                .any(|name| self.advertised.contains_key(*name));
        let requested = config
            .wanted_caps
            .iter()
            .map(String::as_str)
            .chain(config.sasl.as_ref().map(|_| "sasl"))
            .filter(|wanted| *wanted != "ergo.chat/nope")
            .filter(|wanted| *wanted != "sts")
            .filter(|wanted| !config.cap_disabled(wanted))
            .filter(|wanted| *wanted != "metadata-notify" || !modern_metadata)
            .filter_map(|wanted| self.advertised_spelling(wanted))
            .filter(|name| announced.contains(*name))
            .filter(|name| !self.enabled.contains_key(*name))
            .filter(|name| seen.insert((*name).to_owned()))
            .map(str::to_owned)
            .collect();
        self.send_requests(split_requests(requested), outbox);
    }

    fn handle_ls(
        &mut self,
        message: &MessageRef<'_>,
        config: &Config,
        connection: &Connection,
        sasl: &mut SaslSession,
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
        if self.handle_sts(connection, outbox) {
            return;
        }
        self.negotiating = true;

        self.enabled.insert("cap-notify".to_owned(), None);
        let mut seen = BTreeSet::new();
        let modern_metadata = config
            .wanted_caps
            .iter()
            .any(|name| matches!(name.as_str(), "draft/metadata-2" | "draft/metadata-3"))
            && ["draft/metadata-3", "draft/metadata-2"]
                .iter()
                .any(|name| self.advertised.contains_key(*name));
        let wanted = config
            .wanted_caps
            .iter()
            .map(String::as_str)
            .chain(config.sasl.as_ref().map(|_| "sasl"));
        let requested = wanted
            // Ergo disconnects pre-registration clients that request this trap capability.
            .filter(|wanted| *wanted != "ergo.chat/nope")
            .filter(|wanted| *wanted != "sts")
            .filter(|wanted| !config.cap_disabled(wanted))
            .filter(|wanted| *wanted != "metadata-notify" || !modern_metadata)
            .filter_map(|wanted| self.advertised_spelling(wanted))
            .filter(|name| seen.insert((*name).to_owned()))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let requests = split_requests(requested);
        if config.sasl.is_some() && !self.advertised.contains_key("sasl") {
            sasl.unavailable("the server did not advertise sasl", outbox);
        }
        if !requests.is_empty() {
            self.send_requests(requests, outbox);
        }
    }

    fn send_requests(&mut self, requests: Vec<Vec<String>>, outbox: &mut Outbox) {
        for request in requests {
            let mut line = b"CAP REQ :".to_vec();
            line.extend_from_slice(request.join(" ").as_bytes());
            line.extend_from_slice(b"\r\n");
            outbox.transmit_raw(line);
            self.pending.push_back(request);
        }
    }

    fn advertised_spelling(&self, wanted: &str) -> Option<&str> {
        if matches!(wanted, "draft/metadata-2" | "draft/metadata-3") {
            return ["draft/metadata-3", "draft/metadata-2"]
                .into_iter()
                .find_map(|name| {
                    self.advertised
                        .get_key_value(name)
                        .map(|(name, _)| name.as_str())
                });
        }
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

    pub(super) fn maybe_finish(&mut self, authentication_finished: bool, outbox: &mut Outbox) {
        if self.negotiating && authentication_finished && self.pending.is_empty() && !self.ended {
            outbox.transmit_raw(b"CAP END\r\n".to_vec());
            self.ended = true;
        }
    }

    pub(super) const fn enabled(&self) -> &Capabilities {
        &self.enabled
    }

    pub(super) const fn advertised(&self) -> &Capabilities {
        &self.advertised
    }

    pub(super) fn advertised_value(&self, name: &str) -> Option<&Option<String>> {
        self.advertised.get(name)
    }

    pub(super) fn is_pending(&self, name: &str) -> bool {
        self.pending.iter().flatten().any(|pending| pending == name)
    }

    pub(super) const fn is_negotiating(&self) -> bool {
        self.negotiating
    }

    fn handle_sts(&self, connection: &Connection, outbox: &mut Outbox) -> bool {
        let Some(value) = self.advertised.get("sts").and_then(Option::as_deref) else {
            return false;
        };
        let Ok(directive) = StsDirective::parse(value) else {
            return false;
        };
        match decide(directive, connection.security, &connection.host) {
            StsAction::UpgradeTo { port } => {
                outbox.event(Event::StsUpgrade { port });
                outbox.close_and_discard(CloseKind::StsUpgrade, "sts upgrade".to_owned());
                true
            }
            action @ (StsAction::Persist { .. } | StsAction::Remove { .. }) => {
                outbox.event(Event::StsPolicy(action));
                false
            }
            StsAction::Ignore => false,
        }
    }
}

fn split_requests(names: Vec<String>) -> Vec<Vec<String>> {
    let mut requests = Vec::new();
    let mut current = Vec::new();
    let mut current_len = CAP_REQ_PREFIX_LEN;
    for name in names {
        let separator = usize::from(!current.is_empty());
        if !current.is_empty() && current_len + separator + name.len() > MAX_IRC_BODY_LEN {
            requests.push(std::mem::take(&mut current));
            current_len = CAP_REQ_PREFIX_LEN;
        }
        if current_len + name.len() <= MAX_IRC_BODY_LEN {
            current_len += usize::from(!current.is_empty()) + name.len();
            current.push(name);
        }
    }
    if !current.is_empty() {
        requests.push(current);
    }
    requests
}
