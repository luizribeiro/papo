use std::collections::{BTreeSet, VecDeque};
use web_time::Instant;

use papo_proto::{LineRef, MAX_LINE_LEN};

use crate::sts::{StsAction, StsDirective, decide};
use crate::{
    Capabilities, CloseKind, Config, Connection, Event, Handled, Outbox,
    cap::{self, Cap},
    sasl::SaslSession,
};

const CAP_REQ_PREFIX_LEN: usize = b"CAP REQ :".len();
const MAX_IRC_BODY_LEN: usize = MAX_LINE_LEN - 2;

#[derive(Debug, Default)]
pub(super) struct Caps {
    advertised: Capabilities,
    enabled: Capabilities,
    pending: VecDeque<Vec<String>>,
    negotiating: bool,
    ended: bool,
}

impl Caps {
    fn insert_advertised(&mut self, name: &str, value: Option<String>) {
        let name = Cap::from(name);
        let published = cap::request_spelling(name.as_str()) == Some(name.as_str());
        if published
            && self
                .advertised
                .get_key_value(&name)
                .is_some_and(|(existing, _)| existing.as_str() != name.as_str())
        {
            self.advertised.remove(&name);
        }
        self.advertised.insert(name, value);
    }

    pub(super) fn handle_connected(&mut self, _now: Instant) {
        *self = Self::default();
    }

    pub(super) fn handle_message(
        &mut self,
        now: Instant,
        message: &LineRef<'_>,
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
                    let sasl_acked = requested.iter().any(|name| name == cap::SASL.as_str());
                    let isupport_acked =
                        requested.iter().any(|name| name == cap::EXTENDED_ISUPPORT);
                    let mut added = Capabilities::new();
                    for name in requested {
                        let name = Cap::from(name);
                        let value = self.advertised.get(&name).cloned().flatten();
                        self.enabled.insert(name.clone(), value.clone());
                        added.insert(name, value);
                    }
                    if sasl_acked {
                        sasl.handle_cap_ack(
                            now,
                            self.enabled.get(&cap::SASL).and_then(Option::as_deref),
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
                    .is_some_and(|request| request.iter().any(|name| name == cap::SASL.as_str()))
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

    fn handle_del(&mut self, message: &LineRef<'_>, sasl: &mut SaslSession, outbox: &mut Outbox) {
        let mut removed = Capabilities::new();
        let mut sasl_removed = false;
        for name in message.param(2).unwrap_or("").split_whitespace() {
            if name == cap::STS.as_str() {
                continue;
            }
            let name = Cap::from(name);
            self.advertised.remove(&name);
            if let Some(value) = self.enabled.remove(&name) {
                removed.insert(name.clone(), value);
            }
            if name == cap::SASL {
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
        message: &LineRef<'_>,
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
            self.insert_advertised(name, value.clone());
            announced.insert(name.to_owned());
            let name = Cap::from(name);
            if self.enabled.contains_key(&name) {
                self.enabled.insert(name.clone(), value.clone());
                updated.insert(name, value);
            }
        }
        if announced.contains(cap::STS.as_str()) && self.handle_sts(connection, outbox) {
            return;
        }
        if announced.contains(cap::LABELED_RESPONSE.as_str())
            && self.advertised.contains_key(&cap::ECHO_MESSAGE)
        {
            announced.insert(cap::ECHO_MESSAGE.to_string());
        }
        if !updated.is_empty() {
            outbox.event(Event::CapsChanged {
                added: updated,
                removed: Capabilities::new(),
            });
        }

        let mut seen = BTreeSet::new();
        let requested = config
            .wanted_caps
            .iter()
            .map(String::as_str)
            .chain(config.sasl.as_ref().map(|_| cap::SASL.constant_str()))
            .filter(|wanted| cap::requestable(wanted, &self.advertised, &config.wanted_caps))
            .filter(|wanted| !config.cap_disabled(wanted))
            .filter_map(|wanted| cap::advertised_spelling(wanted, &self.advertised))
            .filter(|name| announced.contains(*name))
            .filter(|name| !self.enabled.contains_key(*name))
            .filter(|name| seen.insert((*name).to_owned()))
            .map(str::to_owned)
            .collect();
        self.send_requests(split_requests(requested), outbox);
    }

    fn handle_ls(
        &mut self,
        message: &LineRef<'_>,
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
                self.insert_advertised(name, value.map(str::to_owned));
            }
        }
        if continued {
            return;
        }
        if self.handle_sts(connection, outbox) {
            return;
        }
        if config.sasl.is_none()
            && config.password.is_none()
            && self.advertised.contains_key(&cap::ACCOUNT_REQUIRED)
        {
            outbox.close(
                CloseKind::AccountRequired,
                "the server requires an authenticated account".to_owned(),
            );
            return;
        }
        self.negotiating = true;

        self.enabled.insert(cap::CAP_NOTIFY, None);
        let mut seen = BTreeSet::new();
        let wanted = config
            .wanted_caps
            .iter()
            .map(String::as_str)
            .chain(config.sasl.as_ref().map(|_| cap::SASL.constant_str()));
        let requested = wanted
            .filter(|wanted| cap::requestable(wanted, &self.advertised, &config.wanted_caps))
            .filter(|wanted| !config.cap_disabled(wanted))
            .filter_map(|wanted| cap::advertised_spelling(wanted, &self.advertised))
            .filter(|name| seen.insert((*name).to_owned()))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let requests = split_requests(requested);
        if config.sasl.is_some() && !self.advertised.contains_key(&cap::SASL) {
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

    pub(super) fn maybe_finish(
        &mut self,
        authentication_finished: bool,
        #[cfg_attr(not(feature = "unstable"), allow(unused_variables))] bouncer_network: Option<
            &str,
        >,
        outbox: &mut Outbox,
    ) {
        if self.negotiating && authentication_finished && self.pending.is_empty() && !self.ended {
            #[cfg(feature = "unstable")]
            if self.enabled.contains_key(cap::BOUNCER_NETWORKS)
                && let Some(id) = bouncer_network
            {
                outbox.transmit_immediate("BOUNCER", ["BIND", id]);
            }
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
        self.advertised.get(&Cap::from(name))
    }

    pub(super) fn is_pending(&self, name: &str) -> bool {
        self.pending.iter().flatten().any(|pending| pending == name)
    }

    pub(super) const fn is_negotiating(&self) -> bool {
        self.negotiating
    }

    fn handle_sts(&self, connection: &Connection, outbox: &mut Outbox) -> bool {
        let Some(value) = self.advertised.get(&cap::STS).and_then(Option::as_deref) else {
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
