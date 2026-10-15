use papo_proto::{Isupport, LimitValue};

use crate::Outbox;

const MAX_DESIRED: usize = 10_000;

#[derive(Debug, Default)]
pub(super) struct Monitor {
    desired: Vec<String>,
}

impl Monitor {
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
            outbox.transmit("MONITOR", ["+", &added.join(",")]);
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
            outbox.transmit("MONITOR", ["-", &removed.join(",")]);
        }
    }

    pub(super) fn clear(&mut self, supported: bool, outbox: &mut Outbox) {
        self.desired.clear();
        if supported {
            outbox.transmit("MONITOR", ["C"]);
        }
    }
}
