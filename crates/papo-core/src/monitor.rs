use papo_proto::{Isupport, LimitValue, MAX_MESSAGE_LEN, MessageRef};

use crate::{Event, Handled, Outbox};

const MAX_DESIRED: usize = 10_000;
const MONITOR_PAYLOAD_LEN: usize = MAX_MESSAGE_LEN - "MONITOR + \r\n".len();

#[derive(Debug, Default)]
pub(super) struct Monitor {
    desired: Vec<String>,
    presence: Vec<Presence>,
}

#[derive(Debug, Eq, PartialEq)]
enum Presence {
    Online {
        nick: String,
        user: String,
        host: String,
    },
    Offline(String),
}

impl Monitor {
    pub(super) fn handle_disconnected(&mut self) {
        self.presence.clear();
    }

    pub(super) fn restore(&mut self, isupport: &Isupport, outbox: &mut Outbox) {
        self.presence.clear();
        let Some(limit) = isupport.monitor() else {
            return;
        };
        let limit = match limit {
            LimitValue::Limited(limit) => limit,
            LimitValue::Unlimited => usize::MAX,
        };
        transmit_chunks("+", &self.desired[..self.desired.len().min(limit)], outbox);
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
        let limit = match isupport.monitor() {
            Some(LimitValue::Limited(limit)) => limit,
            Some(LimitValue::Unlimited) => usize::MAX,
            None => 0,
        };
        let mut added = Vec::new();
        for nick in nicks.into_iter().map(Into::into) {
            if self.desired.len() == MAX_DESIRED
                || self.desired.iter().any(|known| mapping.eq(known, &nick))
            {
                continue;
            }
            if registered && self.desired.len() < limit {
                added.push(nick.clone());
            }
            self.desired.push(nick);
        }
        if !added.is_empty() {
            transmit_chunks("+", &added, outbox);
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
        let mut removed = Vec::new();
        for nick in nicks {
            if let Some(index) = self
                .desired
                .iter()
                .position(|known| mapping.eq(known, nick.as_ref()))
            {
                removed.push(self.desired.remove(index));
            }
        }
        if registered && isupport.monitor().is_some() && !removed.is_empty() {
            transmit_chunks("-", &removed, outbox);
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
        match message.raw_command() {
            "730" => {
                let Some(entries) = message.param(1) else {
                    return Handled::No;
                };
                for entry in entries.split(',') {
                    if let Some((nick, user, host)) = parse_mask(entry) {
                        let next = Presence::Online {
                            nick: nick.to_owned(),
                            user: user.to_owned(),
                            host: host.to_owned(),
                        };
                        if self.replace_presence(nick, next, isupport) {
                            outbox.event(Event::MonitorOnline {
                                nick: nick.to_owned(),
                                user: user.to_owned(),
                                host: host.to_owned(),
                            });
                        }
                    }
                }
            }
            "731" => {
                let Some(entries) = message.param(1) else {
                    return Handled::No;
                };
                for nick in entries.split(',').filter(|nick| !nick.is_empty()) {
                    if self.replace_presence(nick, Presence::Offline(nick.to_owned()), isupport) {
                        outbox.event(Event::MonitorOffline {
                            nick: nick.to_owned(),
                        });
                    }
                }
            }
            "734" => {
                let (Some(limit), Some(entries)) = (message.param(1), message.param(2)) else {
                    return Handled::No;
                };
                let Ok(limit) = limit.parse() else {
                    return Handled::No;
                };
                let nicks: Vec<_> = entries.split(',').map(str::to_owned).collect();
                let mapping = isupport.casemapping();
                self.presence
                    .retain(|presence| !nicks.iter().any(|nick| mapping.eq(presence.nick(), nick)));
                outbox.event(Event::MonitorListFull { limit, nicks });
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn replace_presence(&mut self, nick: &str, next: Presence, isupport: &Isupport) -> bool {
        let mapping = isupport.casemapping();
        if let Some(current) = self
            .presence
            .iter_mut()
            .find(|presence| mapping.eq(presence.nick(), nick))
        {
            if *current == next {
                return false;
            }
            *current = next;
        } else {
            self.presence.push(next);
        }
        true
    }
}

impl Presence {
    fn nick(&self) -> &str {
        match self {
            Self::Online { nick, .. } | Self::Offline(nick) => nick,
        }
    }
}

fn parse_mask(mask: &str) -> Option<(&str, &str, &str)> {
    let (nick, userhost) = mask.split_once('!')?;
    let (user, host) = userhost.split_once('@')?;
    (!nick.is_empty() && !user.is_empty() && !host.is_empty()).then_some((nick, user, host))
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
