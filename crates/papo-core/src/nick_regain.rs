use std::time::Duration;

use papo_proto::{Isupport, LineRef};
use web_time::Instant;

use crate::{Config, Monitor, Outbox, names_equal};

const ISON_INTERVAL: Duration = Duration::from_secs(60);

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
    Ison {
        next_poll: Option<Instant>,
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
        now: Instant,
        config: &Config,
        isupport: &Isupport,
        monitor: &Monitor,
        outbox: &mut Outbox,
    ) {
        if !config.regain_nick || !self.primary_taken {
            return;
        }
        if isupport.monitor().is_none() {
            self.watch = Watch::Ison {
                next_poll: Some(now + ISON_INTERVAL),
                claim_pending: false,
            };
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
        now: Instant,
        message: &LineRef<'_>,
        connected: bool,
        current_nick: &str,
        config: &Config,
        isupport: &Isupport,
        monitor: &Monitor,
        outbox: &mut Outbox,
    ) -> bool {
        self.observe_collision(message, connected, &config.nick);
        if matches!(self.watch, Watch::Ison { .. })
            && message.command_is("NICK")
            && names_equal(isupport.casemapping(), current_nick, &config.nick)
        {
            self.reset();
            return false;
        }
        if let Watch::Ison {
            next_poll,
            claim_pending,
        } = &mut self.watch
        {
            return handle_ison(
                now,
                message,
                config,
                isupport,
                next_poll,
                claim_pending,
                outbox,
            );
        }
        let Watch::Monitor {
            added,
            claim_pending,
        } = &mut self.watch
        else {
            return false;
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
        false
    }

    pub(super) fn handle_timeout(&mut self, now: Instant, nick: &str, outbox: &mut Outbox) {
        if let Watch::Ison { next_poll, .. } = &mut self.watch
            && next_poll.is_some_and(|deadline| now >= deadline)
        {
            *next_poll = None;
            outbox.transmit("ISON", [nick]);
        }
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        match self.watch {
            Watch::Ison { next_poll, .. } => next_poll,
            Watch::Inactive | Watch::Monitor { .. } => None,
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

fn unavailable_primary(message: &LineRef<'_>, config: &Config, isupport: &Isupport) -> bool {
    nick_unavailable(message)
        && message
            .param(1)
            .is_some_and(|nick| names_equal(isupport.casemapping(), nick, &config.nick))
}

#[allow(clippy::too_many_arguments)]
fn handle_ison(
    now: Instant,
    message: &LineRef<'_>,
    config: &Config,
    isupport: &Isupport,
    next_poll: &mut Option<Instant>,
    claim_pending: &mut bool,
    outbox: &mut Outbox,
) -> bool {
    if message.command_is("303") && next_poll.is_none() && !*claim_pending {
        let Some(online) = message.param(1) else {
            return false;
        };
        if online
            .split_whitespace()
            .any(|nick| names_equal(isupport.casemapping(), nick, &config.nick))
        {
            *next_poll = Some(now + ISON_INTERVAL);
        } else {
            *claim_pending = true;
            outbox.transmit("NICK", [&config.nick]);
        }
        true
    } else {
        if *claim_pending && unavailable_primary(message, config, isupport) {
            *claim_pending = false;
            *next_poll = Some(now + ISON_INTERVAL);
        }
        false
    }
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
