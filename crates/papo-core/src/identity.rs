use papo_proto::{Command, LineRef, Source};

use crate::Session;

impl Session {
    pub(crate) fn handle_identity_lifecycle(&mut self, message: &LineRef<'_>) {
        match message.command() {
            Command::Nick => {
                let (
                    Some(Source::User {
                        nick,
                        username,
                        host,
                        ..
                    }),
                    Some(new),
                ) = (message.source(), message.param(0))
                else {
                    return;
                };
                if !crate::names_equal(self.names(), nick, self.welcome_state.nick())
                    && crate::names_equal(self.names(), new, self.welcome_state.nick())
                {
                    return;
                }
                if !crate::names_equal(self.names(), nick, new) {
                    self.state.prepare_identity_arrival(new);
                    self.monitor.identity_renamed(
                        nick,
                        new,
                        username,
                        host,
                        self.isupport.value(),
                        &mut self.outbox,
                    );
                }
                self.carry_typing_for_nick(message);
            }
            Command::Quit => {
                if let Some(Source::User { nick, .. }) = message.source() {
                    self.monitor.identity_departed(nick, self.isupport.value());
                }
                self.stop_typing_for_departure(message);
            }
            Command::Part | Command::Kick => self.stop_typing_for_departure(message),
            _ => {}
        }
    }
}
