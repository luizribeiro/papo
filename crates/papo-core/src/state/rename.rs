use std::sync::Arc;

use super::{Channel, Command, Event, LineRef, NameKey, Outbox, State};

pub(super) enum ChannelLifecycle<'a> {
    Joined {
        key: NameKey<String>,
        channel: Arc<Channel>,
    },
    Left(&'a str),
    Renamed {
        old: &'a str,
        new: &'a str,
    },
}

impl State {
    pub(super) fn handle_channel_rename(
        &mut self,
        message: &LineRef<'_>,
        outbox: &mut Outbox,
    ) -> bool {
        if message.command() != Command::Rename || !(2..=3).contains(&message.len()) {
            return false;
        }
        let (Some(actor), Some(old), Some(new)) =
            (message.raw_source(), message.param(0), message.param(1))
        else {
            return false;
        };
        let reason = message
            .param(2)
            .filter(|reason| !reason.is_empty())
            .map(str::to_owned);
        self.channel_lifecycle(ChannelLifecycle::Renamed { old, new });
        outbox.event(Event::ChannelRenamed {
            envelope: crate::LineEnvelope::from_line(message),
            old: self.casemapping.channel(old),
            new: self.casemapping.channel(new),
            actor: actor.to_owned(),
            authorship: self.source_authorship(message),
            reason,
        });
        true
    }

    pub(super) fn channel_lifecycle(&mut self, lifecycle: ChannelLifecycle<'_>) {
        match lifecycle {
            ChannelLifecycle::Joined { key, channel } => {
                self.channels.insert(key, channel);
            }
            ChannelLifecycle::Left(name) => self.forget_channel(name),
            ChannelLifecycle::Renamed { old, new } => self.rename_channel_state(old, new),
        }
    }

    fn forget_channel(&mut self, name: &str) {
        let key = self.casemapping.channel(name);
        let folded = self.casemapping.fold(name);
        self.remove_metadata(name);
        self.who_sweeps.remove(&key);
        crate::typing::TypingTable::forget_channel(&mut self.typing, &key);
        let Some(channel) = self.channels.remove(folded.as_ref()) else {
            return;
        };
        for member in channel.members.values() {
            if let Some(user) = self
                .users
                .get_mut(self.casemapping.fold(&member.nick).as_ref())
            {
                user.channels.remove(&key);
            }
        }
        let orphaned: Vec<_> = self
            .users
            .values()
            .filter(|user| {
                user.nick != self.nick && user.channels.is_empty() && !self.is_monitored(&user.nick)
            })
            .map(|user| user.nick.clone())
            .collect();
        for nick in orphaned {
            self.remove_metadata(&nick);
        }
        let own = self.nick.clone();
        self.users
            .retain(|_, user| user.nick == own || user.monitored || !user.channels.is_empty());
    }

    fn rename_channel_state(&mut self, old: &str, new: &str) {
        let old_channel = self.casemapping.channel(old);
        let new_channel = self.casemapping.channel(new);
        let old_key = NameKey::new(old, self.casemapping);
        let new_key = NameKey::new(new, self.casemapping);

        if let Some(mut channel) = self.channels.remove(&old_key) {
            Arc::make_mut(&mut channel).name = new_channel.clone();
            self.channels.insert(new_key.clone(), channel);
        }
        for user in self.users.values_mut() {
            if user.channels.remove(&old_channel) {
                user.channels.insert(new_channel.clone());
            }
        }
        crate::typing::TypingTable::rename_channel(&mut self.typing, &old_channel, &new_channel);
        if let Some(mut marker) = self.read_markers.remove(&old_key) {
            marker.conversation = crate::Conversation::Channel(new_channel.clone());
            self.read_markers.insert(new_key.clone(), marker);
        }
        self.read_marker_order.replace(&old_key, new_key.clone());
        if let Some(mut seen) = self.last_seen.remove(&old_key) {
            seen.target = new_channel.to_string();
            self.last_seen.insert(new_key.clone(), seen);
        }
        for target in &mut self.last_seen_order {
            if crate::names_equal(self.casemapping, target, old) {
                *target = new_channel.to_string();
            }
        }
        self.rename_metadata(old, new);
        if let Some(count) = self.who_sweeps.remove(&old_channel) {
            self.who_sweeps.insert(new_channel, count);
        }
    }
}
