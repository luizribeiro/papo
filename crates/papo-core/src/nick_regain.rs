use papo_proto::{Isupport, LineRef};

use crate::{Config, Monitor, Outbox, names_equal};

#[derive(Debug, Default)]
pub(super) struct NickRegain {
    primary_taken: bool,
    watch: Watch,
}

#[derive(Debug, Default)]
enum Watch {
    #[default]
    Inactive,
    Monitor {
        added: bool,
        claim_pending: bool,
    },
}

impl NickRegain {
    fn observe_collision(&mut self, message: &LineRef<'_>, connected: bool, primary: &str) {
        if !connected && nick_unavailable(message) && message.param(1) == Some(primary) {
            self.primary_taken = true;
        }
    }

    pub(super) fn start(
        &mut self,
        config: &Config,
        isupport: &Isupport,
        monitor: &Monitor,
        outbox: &mut Outbox,
    ) {
        if !config.regain_nick || !self.primary_taken || isupport.monitor().is_none() {
            return;
        }
        let added = !monitor.desires(&config.nick, isupport);
        if added {
            outbox.transmit("MONITOR", ["+", &config.nick]);
        }
        self.watch = Watch::Monitor {
            added,
            claim_pending: false,
        };
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_message(
        &mut self,
        message: &LineRef<'_>,
        connected: bool,
        current_nick: &str,
        config: &Config,
        isupport: &Isupport,
        monitor: &Monitor,
        outbox: &mut Outbox,
    ) {
        self.observe_collision(message, connected, &config.nick);
        let Watch::Monitor {
            added,
            claim_pending,
        } = &mut self.watch
        else {
            return;
        };
        if message.command_is("NICK")
            && names_equal(isupport.casemapping(), current_nick, &config.nick)
        {
            if *added && !monitor.desires(&config.nick, isupport) {
                outbox.transmit("MONITOR", ["-", &config.nick]);
            }
            self.reset();
        } else if message.command_is("731")
            && message.param(1).is_some_and(|nicks| {
                nicks
                    .split(',')
                    .any(|nick| names_equal(isupport.casemapping(), nick, &config.nick))
            })
            && !*claim_pending
        {
            *claim_pending = true;
            outbox.transmit("NICK", [&config.nick]);
        } else if *claim_pending
            && nick_unavailable(message)
            && message
                .param(1)
                .is_some_and(|nick| names_equal(isupport.casemapping(), nick, &config.nick))
        {
            *claim_pending = false;
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }
}

fn nick_unavailable(message: &LineRef<'_>) -> bool {
    ["433", "436", "437"]
        .iter()
        .any(|numeric| message.command_is(numeric))
}

#[cfg(test)]
mod tests {
    use super::NickRegain;
    use papo_proto::LineRef;

    #[test]
    fn records_only_a_proven_primary_collision_during_registration() {
        for line in [
            ":ergo.test 432 * papo :Erroneous nickname",
            ":ergo.test 433 * another :Nickname is already in use",
        ] {
            let mut regain = NickRegain::default();
            regain.observe_collision(&LineRef::parse(line).unwrap(), false, "papo");
            assert!(!regain.primary_taken);
        }
        let mut regain = NickRegain::default();
        regain.observe_collision(
            &LineRef::parse(":ergo.test 433 * papo :Nickname is already in use").unwrap(),
            false,
            "papo",
        );
        assert!(regain.primary_taken);
        regain.reset();
        assert!(!regain.primary_taken);
    }
}
