use super::{
    Command, Event, LastSeen, LineRef, MAX_LAST_SEEN_TARGETS, NameKey, Outbox, ProtoSource,
    ReadMarker, State, StateLimit, time,
};

impl State {
    pub(super) fn handle_read_marker(
        &mut self,
        message: &LineRef<'_>,
        outbox: &mut Outbox,
    ) -> bool {
        let (Some(target), Some(value)) = (
            message.param(0).filter(|_| message.command_is("MARKREAD")),
            message.param(1).filter(|_| message.len() == 2),
        ) else {
            return false;
        };
        let time = match value {
            "*" | "timestamp=*" => None,
            value => {
                let Some(time) = value.strip_prefix("timestamp=").and_then(time::parse) else {
                    return false;
                };
                Some(time)
            }
        };
        let Ok((status, conversation)) = self.parse_target(target) else {
            return false;
        };
        if status.is_some()
            || !matches!(
                conversation,
                crate::Conversation::Channel(_) | crate::Conversation::Private(_)
            )
        {
            return false;
        }
        let key = NameKey::new(target, self.casemapping);
        let advances = match (self.read_markers.get(&key), time) {
            (None, _)
            | (
                Some(ReadMarker {
                    read_until: None, ..
                }),
                Some(_),
            ) => true,
            (
                Some(ReadMarker {
                    read_until: Some(old),
                    ..
                }),
                Some(new),
            ) => new > *old,
            (Some(_), None) => false,
        };
        if advances {
            let marker = ReadMarker {
                conversation,
                read_until: time,
            };
            if !self.read_markers.contains_key(&key)
                && let Some(evicted) = self.read_marker_order.push_evict_oldest(key.clone())
            {
                self.read_markers.remove(&evicted);
                if self.read_marker_order.take_overflow_warning() {
                    self.warn_limit(StateLimit::ReadMarkers, outbox);
                }
            }
            self.read_markers.insert(key, marker.clone());
            outbox.event(Event::ReadMarkerChanged(marker));
        }
        true
    }

    pub(super) fn remember_last_seen(&mut self, message: &LineRef<'_>, own_nick: &str) {
        if !matches!(message.command(), Command::Privmsg | Command::Notice) {
            return;
        }
        let (Some(mut target), Some(msgid), Some(time)) = (
            message.param(0),
            message.tag("msgid"),
            message.tag("time").and_then(|value| time::parse(&value)),
        ) else {
            return;
        };
        if crate::names_equal(self.casemapping, target, own_nick) {
            let Some(ProtoSource::User { nick, .. }) = message.source() else {
                return;
            };
            target = nick;
        }
        let key = NameKey::new(target, self.casemapping);
        if self
            .last_seen
            .get(&key)
            .is_some_and(|seen| seen.server_time > time)
        {
            return;
        }
        self.last_seen_order
            .retain(|stored| !crate::names_equal(self.casemapping, stored, target));
        if !self.last_seen.contains_key(&key)
            && self.last_seen.len() == MAX_LAST_SEEN_TARGETS
            && let Some(oldest) = self.last_seen_order.pop_front()
        {
            self.last_seen
                .remove(&NameKey::new(oldest, self.casemapping));
        }
        self.last_seen_order.push_back(target.to_owned());
        self.last_seen.insert(
            key,
            LastSeen {
                target: target.to_owned(),
                msgid: msgid.into_owned(),
                server_time: time,
            },
        );
    }
}
