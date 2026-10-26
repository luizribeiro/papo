use super::{Command, Event, LineRef, NameKey, Outbox, State};

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
        let old_key = self.casemapping.channel(old);
        let new_key = self.casemapping.channel(new);
        if let Some(mut channel) = self.channels.remove(&old_key) {
            channel.name = self.casemapping.channel(new);
            self.channels.insert(new_key.clone(), channel);
        }
        for user in self.users.values_mut() {
            if user.channels.remove(&old_key) {
                user.channels.insert(new_key.clone());
            }
        }
        if let Some(count) = self.who_sweeps.remove(&old_key) {
            self.who_sweeps.insert(new_key.clone(), count);
        }
        let old_target = self.casemapping.fold(old);
        if let Some(values) = self.metadata.remove(old_target.as_ref()) {
            self.metadata
                .insert(NameKey::new(new, self.casemapping), values);
        }
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
}
