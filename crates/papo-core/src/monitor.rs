use std::collections::{HashMap, HashSet};

use papo_proto::{Casemapping, Folded, Isupport, LimitValue, MAX_MESSAGE_LEN, MessageRef, Nick};

use crate::{Event, Handled, Outbox};

const MAX_DESIRED: usize = 10_000;
const MONITOR_PAYLOAD_LEN: usize = MAX_MESSAGE_LEN - "MONITOR + \r\n".len();
type NickKey = Folded<Nick>;

#[derive(Debug, Default)]
pub(super) struct Monitor {
    desired: Vec<String>,
    presence: HashMap<NickKey, Presence>,
    casemapping: Casemapping,
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

    pub(super) fn handle_disconnected(&mut self) {
        self.presence.clear();
    }

    pub(super) fn restore(&mut self, isupport: &Isupport, outbox: &mut Outbox) {
        self.presence.clear();
        let Some(limit) = isupport.monitor() else {
            if !self.desired.is_empty() {
                self.warn_unsupported(outbox);
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
                nicks: self.desired[limit..].to_vec(),
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
        let mapping = isupport.casemapping();
        self.update_casemapping(mapping);
        let support = isupport.monitor();
        let limit = match support {
            Some(LimitValue::Limited(limit)) => limit,
            Some(LimitValue::Unlimited) => usize::MAX,
            None => 0,
        };
        let mut added = Vec::new();
        let mut unavailable = Vec::new();
        let mut full = Vec::new();
        let mut known: HashSet<_> = self
            .desired
            .iter()
            .map(|nick| NickKey::new(nick, mapping))
            .collect();
        for nick in nicks.into_iter().map(Into::into) {
            if !known.insert(NickKey::new(nick.as_str(), mapping)) {
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
        if registered && support.is_none() {
            self.warn_unsupported(outbox);
        }
        if !unavailable.is_empty() {
            outbox.event(Event::MonitorListFull {
                limit,
                nicks: unavailable,
            });
        }
        if !full.is_empty() {
            outbox.event(Event::MonitorListFull {
                limit: MAX_DESIRED,
                nicks: full,
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
        let mapping = isupport.casemapping();
        self.update_casemapping(mapping);
        let limit = match isupport.monitor() {
            Some(LimitValue::Limited(limit)) => limit,
            Some(LimitValue::Unlimited) => usize::MAX,
            None => 0,
        };
        let old: Vec<_> = self.desired.iter().take(limit).cloned().collect();
        for nick in nicks {
            let nick = nick.as_ref();
            if let Some(index) = self
                .desired
                .iter()
                .position(|known| mapping.eq(known, nick))
            {
                self.desired.remove(index);
                self.presence.remove(&NickKey::new(nick, mapping));
            }
        }
        if registered && isupport.monitor().is_some() {
            let new: Vec<_> = self.desired.iter().take(limit).cloned().collect();
            let removed: Vec<_> = old
                .iter()
                .filter(|nick| !new.iter().any(|new| mapping.eq(nick, new)))
                .cloned()
                .collect();
            let added: Vec<_> = new
                .iter()
                .filter(|nick| !old.iter().any(|old| mapping.eq(nick, old)))
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
        message: &MessageRef<'_>,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> Handled {
        self.update_casemapping(isupport.casemapping());
        match message.raw_command() {
            "730" => {
                let Some(entries) = message.param(1) else {
                    return Handled::No;
                };
                let mut handled = false;
                for entry in entries.split(',') {
                    if let Some((nick, user, host)) = parse_entry(entry) {
                        handled = true;
                        let (was_online, userhost_changed) = {
                            let previous = self
                                .presence
                                .get(&NickKey::new(nick, isupport.casemapping()))
                                .and_then(|presence| {
                                    let Presence::Online {
                                        nick: _,
                                        user,
                                        host,
                                    } = presence
                                    else {
                                        return None;
                                    };
                                    Some((user.as_deref(), host.as_deref()))
                                });
                            (
                                previous.is_some(),
                                previous.is_some_and(|old| old != (user, host)),
                            )
                        };
                        let next = Presence::Online {
                            nick: nick.to_owned(),
                            user: user.map(str::to_owned),
                            host: host.map(str::to_owned),
                        };
                        self.replace_presence(nick, next, isupport);
                        if !was_online {
                            outbox.event(Event::MonitorOnline {
                                nick: nick.to_owned(),
                                user: user.map(str::to_owned),
                                host: host.map(str::to_owned),
                            });
                        } else if userhost_changed && let (Some(user), Some(host)) = (user, host) {
                            outbox.event(Event::HostChanged {
                                nick: nick.to_owned(),
                                user: user.to_owned(),
                                host: host.to_owned(),
                            });
                        }
                    }
                }
                if !handled {
                    return Handled::No;
                }
            }
            "731" => {
                let Some(entries) = message.param(1) else {
                    return Handled::No;
                };
                let mut handled = false;
                for nick in entries.split(',').filter(|nick| !nick.is_empty()) {
                    handled = true;
                    if self.replace_presence(nick, Presence::Offline(nick.to_owned()), isupport) {
                        outbox.event(Event::MonitorOffline {
                            nick: nick.to_owned(),
                        });
                    }
                }
                if !handled {
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
                let mapping = isupport.casemapping();
                for nick in &nicks {
                    self.presence.remove(&NickKey::new(nick, mapping));
                }
                outbox.event(Event::MonitorListFull { limit, nicks });
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn replace_presence(&mut self, nick: &str, next: Presence, isupport: &Isupport) -> bool {
        let mapping = isupport.casemapping();
        let key = NickKey::new(nick, mapping);
        if let Some(current) = self.presence.get_mut(&key) {
            if *current == next {
                return false;
            }
            *current = next;
        } else {
            self.presence.insert(key, next);
        }
        true
    }

    fn update_casemapping(&mut self, mapping: Casemapping) {
        if self.casemapping == mapping {
            return;
        }
        self.casemapping = mapping;
        self.presence = std::mem::take(&mut self.presence)
            .into_values()
            .map(|presence| (NickKey::new(presence.nick(), mapping), presence))
            .collect();
    }

    fn warn_unsupported(&mut self, outbox: &mut Outbox) {
        if !self.unsupported_warned {
            self.unsupported_warned = true;
            outbox.event(Event::MonitorUnsupported);
        }
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
