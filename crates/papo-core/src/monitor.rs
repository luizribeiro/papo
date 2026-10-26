use std::collections::{BTreeMap, HashSet};

use papo_proto::{Advertised, Casemapping, Isupport, LimitValue, LineRef, MAX_LINE_LEN, Nick};

use crate::{Authorship, Event, Feature, Handled, Names, Outbox, Unsupported, UnsupportedReason};

const MAX_DESIRED: usize = 10_000;
const MONITOR_PAYLOAD_LEN: usize = MAX_LINE_LEN - "MONITOR + \r\n".len();
type NickKey = Nick;

#[derive(Debug, Default)]
pub(super) struct Monitor {
    desired: Vec<String>,
    presence: BTreeMap<NickKey, Presence>,
    casemapping: Names,
    unsupported_warned: bool,
}

#[derive(Debug, Eq, PartialEq)]
enum Presence {
    Online {
        nick: String,
        user: Option<String>,
        host: Option<String>,
    },
    Offline(String),
}

impl Monitor {
    pub(super) fn new(origin: papo_proto::Origin) -> Self {
        Self {
            casemapping: Names::new(Casemapping::default(), origin),
            ..Self::default()
        }
    }

    pub(super) fn desires(&self, nick: &str, isupport: &Isupport) -> bool {
        self.desired
            .iter()
            .any(|known| crate::names_equal(isupport.casemapping(), known, nick))
    }

    pub(super) fn desired(&self) -> impl Iterator<Item = &str> {
        self.desired.iter().map(String::as_str)
    }

    pub(super) fn online(&self) -> impl Iterator<Item = (&str, Option<&str>, Option<&str>)> {
        self.presence
            .values()
            .filter_map(|presence| match presence {
                Presence::Online { nick, user, host } => {
                    Some((nick.as_str(), user.as_deref(), host.as_deref()))
                }
                Presence::Offline(_) => None,
            })
    }

    pub(super) fn online_nick(
        &self,
        nick: &str,
        _isupport: &Isupport,
    ) -> Option<(Option<&str>, Option<&str>)> {
        match self.presence.get(&self.casemapping.nick(nick))? {
            Presence::Online { user, host, .. } => Some((user.as_deref(), host.as_deref())),
            Presence::Offline(_) => None,
        }
    }

    pub(super) fn handle_disconnected(&mut self) {
        self.presence.clear();
    }

    pub(super) fn restore(&mut self, isupport: &Isupport, outbox: &mut Outbox) {
        self.presence.clear();
        if matches!(isupport.get("MONITOR"), Advertised::Value("0")) {
            if !self.desired.is_empty() {
                self.warn_unsupported(UnsupportedReason::LimitZero, outbox);
            }
            return;
        }
        let Some(limit) = isupport.monitor() else {
            if !self.desired.is_empty() {
                self.warn_unsupported(UnsupportedReason::NotOffered, outbox);
            }
            return;
        };
        let limit = match limit {
            LimitValue::Limited(limit) => limit,
            LimitValue::Unlimited => usize::MAX,
        };
        transmit_chunks("+", &self.desired[..self.desired.len().min(limit)], outbox);
        if self.desired.len() > limit {
            outbox.event(Event::MonitorListFull {
                limit,
                nicks: self.desired[limit..]
                    .iter()
                    .map(|nick| self.casemapping.nick(nick))
                    .collect(),
            });
        }
    }

    pub(super) fn add<I, S>(
        &mut self,
        nicks: I,
        isupport: &Isupport,
        registered: bool,
        outbox: &mut Outbox,
    ) where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mapping = self.names(isupport);
        self.update_casemapping(mapping.casemapping());
        let support = isupport.monitor();
        let limit = match support {
            Some(LimitValue::Limited(limit)) => limit,
            Some(LimitValue::Unlimited) => usize::MAX,
            None => 0,
        };
        let mut added = Vec::new();
        let mut unavailable = Vec::new();
        let mut full = Vec::new();
        let mut known: HashSet<_> = self.desired.iter().map(|nick| mapping.nick(nick)).collect();
        for nick in nicks.into_iter().map(Into::into) {
            let key = mapping.nick(nick.as_str());
            if key.check(isupport).is_err() || !known.insert(key) {
                continue;
            }
            if self.desired.len() == MAX_DESIRED {
                full.push(nick);
                continue;
            }
            if registered && self.desired.len() < limit {
                added.push(nick.clone());
            } else if registered && support.is_some() {
                unavailable.push(nick.clone());
            }
            self.desired.push(nick);
        }
        if !added.is_empty() {
            transmit_chunks("+", &added, outbox);
        }
        if !unavailable.is_empty() {
            outbox.event(Event::MonitorListFull {
                limit,
                nicks: unavailable
                    .into_iter()
                    .map(|nick| mapping.nick(nick))
                    .collect(),
            });
        }
        if !full.is_empty() {
            outbox.event(Event::MonitorListFull {
                limit: MAX_DESIRED,
                nicks: full.into_iter().map(|nick| mapping.nick(nick)).collect(),
            });
        }
    }

    pub(super) fn remove<I, S>(
        &mut self,
        nicks: I,
        isupport: &Isupport,
        registered: bool,
        outbox: &mut Outbox,
    ) where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mapping = self.names(isupport);
        self.update_casemapping(mapping.casemapping());
        let limit = match isupport.monitor() {
            Some(LimitValue::Limited(limit)) => limit,
            Some(LimitValue::Unlimited) => usize::MAX,
            None => 0,
        };
        let old: Vec<_> = self.desired.iter().take(limit).cloned().collect();
        for nick in nicks {
            let nick = nick.as_ref();
            if mapping.nick(nick).check(isupport).is_err() {
                continue;
            }
            if let Some(index) = self
                .desired
                .iter()
                .position(|known| crate::names_equal(mapping, known, nick))
            {
                self.desired.remove(index);
                self.presence.remove(&mapping.nick(nick));
            }
        }
        if registered && isupport.monitor().is_some() {
            let new: Vec<_> = self.desired.iter().take(limit).cloned().collect();
            let removed: Vec<_> = old
                .iter()
                .filter(|nick| !new.iter().any(|new| crate::names_equal(mapping, nick, new)))
                .cloned()
                .collect();
            let added: Vec<_> = new
                .iter()
                .filter(|nick| !old.iter().any(|old| crate::names_equal(mapping, nick, old)))
                .cloned()
                .collect();
            transmit_chunks("-", &removed, outbox);
            transmit_chunks("+", &added, outbox);
        }
    }

    pub(super) fn clear(&mut self, supported: bool, outbox: &mut Outbox) {
        self.desired.clear();
        self.presence.clear();
        if supported {
            outbox.transmit("MONITOR", ["C"]);
        }
    }

    pub(super) fn handle_message(
        &mut self,
        message: &LineRef<'_>,
        isupport: &Isupport,
        internal_nick: Option<&str>,
        outbox: &mut Outbox,
    ) -> Handled {
        self.update_casemapping(isupport.casemapping());
        match message.raw_command() {
            "730" => {
                let Some(entries) = message.param(1) else {
                    return Handled::No;
                };
                if !self.handle_online(
                    entries,
                    isupport,
                    internal_nick,
                    crate::LineEnvelope::from_line(message).server_time(),
                    outbox,
                ) {
                    return Handled::No;
                }
            }
            "731" => {
                let Some(entries) = message.param(1) else {
                    return Handled::No;
                };
                if !self.handle_offline(entries, isupport, internal_nick, outbox) {
                    return Handled::No;
                }
            }
            "734" => {
                let (Some(limit), Some(entries)) = (message.param(1), message.param(2)) else {
                    return Handled::No;
                };
                let Ok(limit) = limit.parse() else {
                    return Handled::No;
                };
                let nicks: Vec<_> = entries
                    .split(',')
                    .filter(|nick| !nick.is_empty())
                    .map(str::to_owned)
                    .collect();
                if nicks.is_empty() {
                    return Handled::No;
                }
                let mapping = self.names(isupport);
                for nick in &nicks {
                    self.presence.remove(&mapping.nick(nick));
                }
                outbox.event(Event::MonitorListFull {
                    limit,
                    nicks: nicks.into_iter().map(|nick| mapping.nick(nick)).collect(),
                });
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn handle_online(
        &mut self,
        entries: &str,
        isupport: &Isupport,
        internal_nick: Option<&str>,
        server_time: Option<std::time::SystemTime>,
        outbox: &mut Outbox,
    ) -> bool {
        let mut handled = false;
        for entry in entries.split(',') {
            let Some((nick, user, host)) = parse_entry(entry) else {
                continue;
            };
            handled = true;
            if !self.accepts_presence(nick, isupport, internal_nick) {
                continue;
            }
            let previous = self
                .presence
                .get(&self.casemapping.nick(nick))
                .and_then(|presence| match presence {
                    Presence::Online { user, host, .. } => Some((user.as_deref(), host.as_deref())),
                    Presence::Offline(_) => None,
                });
            let was_online = previous.is_some();
            let userhost_changed = previous.is_some_and(|old| old != (user, host));
            let next = Presence::Online {
                nick: nick.to_owned(),
                user: user.map(str::to_owned),
                host: host.map(str::to_owned),
            };
            let changed = self.replace_presence(nick, next, isupport);
            if !was_online && changed {
                outbox.event(Event::MonitorOnline {
                    nick: self.casemapping.nick(nick),
                    username: user.map(str::to_owned),
                    host: host.map(str::to_owned),
                });
            } else if userhost_changed && let (Some(user), Some(host)) = (user, host) {
                outbox.event(Event::HostChanged {
                    server_time,
                    actor: nick.to_owned(),
                    nick: self.casemapping.nick(nick),
                    username: user.to_owned(),
                    host: host.to_owned(),
                    authorship: if internal_nick
                        .is_some_and(|own| crate::names_equal(isupport.casemapping(), own, nick))
                    {
                        Authorship::Own
                    } else {
                        Authorship::Other
                    },
                });
            }
        }
        handled
    }

    fn handle_offline(
        &mut self,
        entries: &str,
        isupport: &Isupport,
        internal_nick: Option<&str>,
        outbox: &mut Outbox,
    ) -> bool {
        let mut handled = false;
        for nick in entries.split(',').filter(|nick| !nick.is_empty()) {
            handled = true;
            if self.accepts_presence(nick, isupport, internal_nick)
                && self.replace_presence(nick, Presence::Offline(nick.to_owned()), isupport)
            {
                outbox.event(Event::MonitorOffline {
                    nick: self.casemapping.nick(nick),
                });
            }
        }
        handled
    }

    fn accepts_presence(
        &self,
        nick: &str,
        isupport: &Isupport,
        internal_nick: Option<&str>,
    ) -> bool {
        self.desires(nick, isupport)
            || internal_nick
                .is_some_and(|known| crate::names_equal(isupport.casemapping(), known, nick))
    }

    fn replace_presence(&mut self, nick: &str, next: Presence, isupport: &Isupport) -> bool {
        let mapping = self.names(isupport);
        let key = mapping.nick(nick);
        if let Some(current) = self.presence.get_mut(&key) {
            if *current == next {
                return false;
            }
            *current = next;
        } else if self.presence.len() < MAX_DESIRED {
            self.presence.insert(key, next);
        } else {
            return false;
        }
        true
    }

    fn names(&self, isupport: &Isupport) -> Names {
        Names::new(isupport.casemapping(), self.casemapping.origin())
    }

    fn update_casemapping(&mut self, mapping: Casemapping) {
        if self.casemapping.casemapping() == mapping {
            return;
        }
        self.casemapping = Names::new(mapping, self.casemapping.origin());
        let names = self.casemapping;
        self.presence = std::mem::take(&mut self.presence)
            .into_values()
            .map(|presence| (names.nick(presence.nick()), presence))
            .collect();
    }

    fn warn_unsupported(&mut self, reason: UnsupportedReason, outbox: &mut Outbox) {
        if !self.unsupported_warned {
            self.unsupported_warned = true;
            outbox.event(Event::Unsupported(Unsupported::new(
                Feature::Monitor,
                reason,
            )));
        }
    }
}

pub(super) fn presence_nicks<'a>(message: &LineRef<'a>) -> Vec<&'a str> {
    match message.raw_command() {
        "730" => message
            .param(1)
            .into_iter()
            .flat_map(|entries| entries.split(','))
            .filter_map(|entry| parse_entry(entry).map(|(nick, _, _)| nick))
            .collect(),
        "731" => message
            .param(1)
            .into_iter()
            .flat_map(|entries| entries.split(','))
            .filter(|nick| !nick.is_empty())
            .collect(),
        "734" => message
            .param(2)
            .into_iter()
            .flat_map(|entries| entries.split(','))
            .filter(|nick| !nick.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

impl Presence {
    fn nick(&self) -> &str {
        match self {
            Self::Online { nick, .. } | Self::Offline(nick) => nick,
        }
    }
}

fn parse_entry(entry: &str) -> Option<(&str, Option<&str>, Option<&str>)> {
    let Some((nick, userhost)) = entry.split_once('!') else {
        return (!entry.is_empty()).then_some((entry, None, None));
    };
    let (user, host) = userhost.split_once('@')?;
    (!nick.is_empty() && !user.is_empty() && !host.is_empty()).then_some((
        nick,
        Some(user),
        Some(host),
    ))
}

fn transmit_chunks(operation: &str, nicks: &[String], outbox: &mut Outbox) {
    let mut joined = String::new();
    for nick in nicks
        .iter()
        .filter(|nick| nick.len() <= MONITOR_PAYLOAD_LEN)
    {
        if !joined.is_empty() && joined.len() + 1 + nick.len() > MONITOR_PAYLOAD_LEN {
            outbox.transmit("MONITOR", [operation, &joined]);
            joined.clear();
        }
        if !joined.is_empty() {
            joined.push(',');
        }
        joined.push_str(nick);
    }
    if !joined.is_empty() {
        outbox.transmit("MONITOR", [operation, &joined]);
    }
}
