use super::{Command, Event, Folded, LineRef, Outbox, State};

impl State {
    pub(super) fn handle_channel_rename(
        &mut self,
        message: &LineRef<'_>,
        outbox: &mut Outbox,
    ) -> bool {
        if message.command() != Command::Rename || message.len() != 3 {
            return false;
        }
        let (Some(old), Some(new), Some(reason)) =
            (message.param(0), message.param(1), message.param(2))
        else {
            return false;
        };
        let old_key = Folded::new(old, self.casemapping);
        let new_key = Folded::new(new, self.casemapping);
        if let Some(mut channel) = self.channels.remove(&old_key) {
            channel.name = new.into();
            self.channels.insert(new_key.clone(), channel);
        }
        for user in self.users.values_mut() {
            if user.channels.remove(&old_key) {
                user.channels.insert(new_key.clone());
            }
        }
        if self.who_sweeps.remove(&old_key) {
            self.who_sweeps.insert(new_key.clone());
        }
        let old_target = self.casemapping.fold(old);
        if let Some(values) = self.metadata.remove(old_target.as_ref()) {
            self.metadata
                .insert(Folded::new(new, self.casemapping), values);
        }
        outbox.event(Event::ChannelRenamed {
            old: old.to_owned(),
            new: new.to_owned(),
            reason: reason.to_owned(),
        });
        true
    }
}
