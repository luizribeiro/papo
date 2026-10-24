use std::collections::{BTreeSet, VecDeque};
use web_time::Instant;

use papo_proto::{Line, LineRef, MAX_LINE_LEN};

#[cfg(test)]
use crate::sts::ConnectionSecurity;
use crate::sts::{StsAction, StsDirective, decide};
use crate::{
    Capabilities, CloseKind, Config, ConnectPhase, Connection, Event, Handled, Outbox,
    RegistrationPolicy,
    cap::{self, Cap},
    sasl::SaslSession,
};

const CAP_REQ_PREFIX_LEN: usize = b"CAP REQ :".len();
const MAX_IRC_BODY_LEN: usize = MAX_LINE_LEN - 2;
const MAX_ADVERTISED: usize = 512;
const MAX_CAP_VALUE_LEN: usize = 4_096;
const MAX_LS_LINES: usize = 512;

#[derive(Debug, Default)]
pub(super) struct Caps {
    advertised: Capabilities,
    enabled: Capabilities,
    pending: VecDeque<Vec<String>>,
    negotiating: bool,
    ended: bool,
    ls_lines: usize,
}

impl Caps {
    fn insert_advertised(
        &mut self,
        name: &str,
        mut value: Option<String>,
        protected: bool,
        allow_new: bool,
    ) {
        let name = Cap::from(name);
        if !self.advertised.contains_key(&name) && !allow_new {
            return;
        }
        if !self.advertised.contains_key(&name) && self.advertised.len() == MAX_ADVERTISED {
            if !protected {
                return;
            }
            let Some(victim) = self.advertised.keys().next().cloned() else {
                return;
            };
            self.advertised.remove(&victim);
        }
        if let Some(value) = &mut value {
            let mut boundary = value.len().min(MAX_CAP_VALUE_LEN);
            while !value.is_char_boundary(boundary) {
                boundary -= 1;
            }
            value.truncate(boundary);
        }
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
                    let isupport_acked = requested
                        .iter()
                        .any(|name| name == cap::EXTENDED_ISUPPORT.as_str());
                    let mut added = Capabilities::new();
                    for name in requested {
                        let name = Cap::from(name);
                        let value = self.advertised.get(&name).cloned().flatten();
                        self.enabled.insert(name.clone(), value.clone());
                        added.insert(name, value);
                    }
                    if sasl_acked {
                        if !self.ended {
                            outbox.connect_phase(ConnectPhase::Authenticating);
                        }
                        sasl.handle_cap_ack(
                            now,
                            connection.security,
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
                if let Some(requested) = self.pending.pop_front() {
                    if requested.iter().any(|name| name == cap::SASL.as_str()) {
                        sasl.unavailable("the server rejected the sasl capability", outbox);
                    } else if let Some(required) = required_in(config, &requested) {
                        close_required(&required, "was rejected", outbox);
                    }
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
            self.insert_advertised(name, value.clone(), wanted(config, name), true);
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
            .filter(|wanted| !cap::is_unstable(wanted))
            .filter(|wanted| {
                cap::requestable(wanted, &self.advertised, &config.wanted_caps, config)
            })
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
        self.ls_lines = self.ls_lines.saturating_add(1);
        for token in list.split_whitespace() {
            let (name, value) = token
                .split_once('=')
                .map_or((token, None), |(name, value)| (name, Some(value)));
            if !name.is_empty() {
                let protected = wanted(config, name);
                self.insert_advertised(
                    name,
                    value.map(str::to_owned),
                    protected,
                    self.ls_lines <= MAX_LS_LINES || protected,
                );
            }
        }
        if continued {
            return;
        }
        self.ls_lines = 0;
        if self.handle_sts(connection, outbox) {
            return;
        }
        if connection.security == crate::sts::ConnectionSecurity::Plaintext
            && config.password.is_some()
            && !config.allows_insecure_authentication()
        {
            close_required(&cap::STS, "did not offer a valid TLS upgrade", outbox);
            return;
        }
        if let Some(required) = config
            .required_capabilities()
            .into_iter()
            .find(|required| !self.advertised.contains_key(required))
        {
            close_required(&required, "was not advertised", outbox);
            return;
        }
        if config.creates_account_on_connect()
            && !RegistrationPolicy::parse(
                self.advertised
                    .get(&cap::ACCOUNT_REGISTRATION)
                    .and_then(Option::as_deref),
            )
            .before_connect
        {
            close_required_value(&cap::ACCOUNT_REGISTRATION, "before-connect", outbox);
            return;
        }
        if let Some(required) = config
            .required_capabilities()
            .into_iter()
            .find(|required| config.cap_disabled(required.as_str()))
        {
            close_required(&required, "was disabled by the client", outbox);
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
            .filter(|wanted| {
                cap::requestable(wanted, &self.advertised, &config.wanted_caps, config)
            })
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
            let line = Line::new("CAP").param("REQ").param(request.join(" "));
            if let Ok(line) = line.encode() {
                outbox.transmit_raw(line);
            }
            self.pending.push_back(request);
        }
    }

    pub(super) fn maybe_finish(
        &mut self,
        authentication_finished: bool,
        config: &Config,
        outbox: &mut Outbox,
    ) {
        if self.negotiating
            && authentication_finished
            && self.pending.is_empty()
            && !self.ended
            && !outbox.close_requested
        {
            if let Some(required) = config
                .required_capabilities()
                .into_iter()
                .find(|required| !self.enabled.contains_key(required))
            {
                close_required(&required, "was not acknowledged", outbox);
                return;
            }
            if let Some(id) = config.selected_bouncer_network() {
                if !self.bouncer_enabled() {
                    outbox.close(
                        crate::CloseKind::BouncerBindingFailed,
                        "selected bouncer network requires acknowledged soju.im/bouncer-networks"
                            .to_owned(),
                    );
                    return;
                }
                outbox.transmit_immediate("BOUNCER", ["BIND", id]);
            }
            outbox.connect_phase(ConnectPhase::WaitingForWelcome);
            if let Ok(line) = Line::new("CAP").param("END").encode() {
                outbox.transmit_raw(line);
            }
            self.ended = true;
        }
    }

    pub(super) const fn enabled(&self) -> &Capabilities {
        &self.enabled
    }

    pub(super) fn bouncer_enabled(&self) -> bool {
        cfg!(feature = "unstable") && self.enabled.contains_key(cap::BOUNCER_NETWORKS_NAME)
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

fn required_in(config: &Config, requested: &[String]) -> Option<Cap> {
    let required = config.required_capabilities();
    requested
        .iter()
        .find(|name| required.contains(&Cap::from(name.as_str())))
        .map(|name| Cap::from(name.as_str()))
}

fn close_required(required: &Cap, reason: &str, outbox: &mut Outbox) {
    outbox.close(
        CloseKind::CapabilityUnavailable,
        format!(
            "required IRC capability {} {reason}",
            cap::canonical(required.as_str())
        ),
    );
}

fn close_required_value(required: &Cap, value: &str, outbox: &mut Outbox) {
    outbox.close(
        CloseKind::CapabilityUnavailable,
        format!(
            "required IRC capability {} does not offer {value}",
            cap::canonical(required.as_str())
        ),
    );
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

fn wanted(config: &Config, name: &str) -> bool {
    config
        .wanted_caps
        .iter()
        .any(|wanted| cap::canonical(wanted) == cap::canonical(name))
        || (config.sasl.is_some() && cap::canonical(name) == cap::SASL.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertised_capabilities_stay_bounded_and_keep_wanted_caps() {
        let now = Instant::now();
        let config = Config::default().wanted_caps(["vendor.example/wanted"]);
        let connection = Connection {
            host: "irc.example.net".to_owned(),
            security: ConnectionSecurity::Secure,
        };
        let mut caps = Caps::default();
        let mut sasl = SaslSession::new(&config, None);
        let mut outbox = Outbox::default();
        for index in 0..10_000 {
            let wire = format!("CAP papo NEW :vendor.example/cap-{index}");
            caps.handle_message(
                now,
                &LineRef::parse(&wire).unwrap(),
                &config,
                &connection,
                &mut sasl,
                &mut outbox,
            );
        }
        let wanted = LineRef::parse("CAP papo NEW :vendor.example/wanted").unwrap();
        caps.handle_message(now, &wanted, &config, &connection, &mut sasl, &mut outbox);

        assert_eq!(caps.advertised.len(), 512);
        assert!(
            caps.pending
                .iter()
                .flatten()
                .any(|name| name == "vendor.example/wanted")
        );
    }

    #[test]
    fn advertised_caps_preserve_values_and_prefer_published_spellings() {
        let mut caps = Caps::default();
        caps.insert_advertised("server-time", Some("iso8601".to_owned()), false, true);
        assert_eq!(
            caps.advertised.get("server-time"),
            Some(&Some("iso8601".to_owned()))
        );

        caps.insert_advertised("chathistory", Some("legacy-value".to_owned()), false, true);
        caps.insert_advertised("draft/chathistory", None, false, true);
        let (name, value) = caps.advertised.get_key_value(&cap::CHATHISTORY).unwrap();
        assert_eq!(name.as_str(), "draft/chathistory");
        assert_eq!(value, &None);
    }

    #[test]
    fn advertised_cap_updates_obey_new_and_protected_rules() {
        let mut caps = Caps::default();
        caps.insert_advertised("server-time", Some("old".to_owned()), false, true);
        caps.insert_advertised("server-time", Some("iso8601".to_owned()), false, false);
        caps.insert_advertised("vendor.example/unannounced", None, false, false);
        assert_eq!(caps.advertised.len(), 1);
        assert_eq!(caps.advertised["server-time"].as_deref(), Some("iso8601"));

        for (count, retained) in [(511, 511), (512, 512), (513, 512)] {
            let mut bounded = Caps::default();
            for index in 0..count {
                bounded.insert_advertised(
                    &format!("vendor.example/cap-{index}"),
                    None,
                    false,
                    true,
                );
            }
            assert_eq!(bounded.advertised.len(), retained, "input count {count}");
        }

        for index in 0..512 {
            caps.insert_advertised(&format!("vendor.example/cap-{index}"), None, false, true);
        }
        caps.insert_advertised("vendor.example/overflow", None, false, true);
        assert_eq!(caps.advertised.len(), 512);
        assert!(!caps.advertised.contains_key("vendor.example/overflow"));

        caps.insert_advertised("sasl", Some("PLAIN".to_owned()), true, true);
        assert_eq!(caps.advertised.len(), 512);
        assert_eq!(caps.advertised["sasl"].as_deref(), Some("PLAIN"));
    }

    #[test]
    fn advertised_cap_values_keep_literal_byte_boundaries() {
        for (value, expected) in [
            ("x".repeat(4_095), "x".repeat(4_095)),
            ("x".repeat(4_096), "x".repeat(4_096)),
            ("x".repeat(4_097), "x".repeat(4_096)),
            (format!("{}é", "x".repeat(4_095)), "x".repeat(4_095)),
        ] {
            let mut caps = Caps::default();
            caps.insert_advertised("vendor.example/value", Some(value), false, true);
            assert_eq!(
                caps.advertised["vendor.example/value"].as_deref(),
                Some(expected.as_str())
            );
        }
    }

    #[test]
    fn wanted_caps_include_only_configured_names_and_sasl() {
        let configured = Config::default().wanted_caps(["draft/chathistory"]);
        let credentials = Config::default().sasl(crate::Sasl::plain("alice", "hunter2"));
        for (config, name, expected) in [
            (&configured, "chathistory", true),
            (&configured, "vendor.example/unwanted", false),
            (&credentials, "sasl", true),
            (&credentials, "vendor.example/unwanted", false),
        ] {
            assert_eq!(wanted(config, name), expected, "{name}");
        }
    }
}
