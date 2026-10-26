use std::{collections::VecDeque, sync::Arc, time::Duration};

use papo_proto::{Command, LineRef, Nick, Prefix, Source, Typing};
use web_time::Instant;

use crate::{
    Authorship, Conversation, Event, Session,
    conversation::{LineTarget, classify_line_with_names},
};

const MAX_TYPERS_PER_CONVERSATION: usize = 32;
const MAX_TYPERS: usize = 512;

/// Why a user's typing state changed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum TypingReason {
    /// The user sent a typing notification.
    Notified,
    /// The locally measured typing deadline elapsed.
    TimedOut,
    /// The user sent a chat message to the conversation.
    Messaged,
    /// The user left the conversation or disconnected.
    Departed,
}

/// A user's typing state changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TypingChanged {
    /// The user prefix associated with the typing state.
    pub actor: String,
    /// Whether the user is this session's user.
    pub authorship: Authorship,
    /// The IRC conversation where typing is occurring.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub conversation: Conversation,
    /// The lowest-ranked `STATUSMSG` prefix on the typing notification's wire address.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub status: Option<Prefix>,
    return_address: String,
    /// The new typing state.
    pub state: Typing,
    /// Why the state changed.
    pub reason: TypingReason,
}

impl TypingChanged {
    /// Returns the address that reaches the recipients of the typing notification.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn return_address(&self) -> String {
        self.return_address.clone()
    }
}

/// A user currently composing a message in a conversation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Typer {
    nick: Nick,
    conversation: Conversation,
    state: Typing,
    status: Option<Prefix>,
    return_address: String,
}

impl Typer {
    /// Returns the user's nickname.
    #[must_use]
    pub const fn nick(&self) -> &Nick {
        &self.nick
    }

    /// Returns whether the user has paused rather than actively typing.
    #[must_use]
    pub const fn is_paused(&self) -> bool {
        matches!(self.state, Typing::Paused)
    }

    /// Returns the lowest-ranked `STATUSMSG` prefix on the typing notification's wire address.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub const fn status(&self) -> Option<Prefix> {
        self.status
    }

    /// Returns the address that reaches the recipients of the typing notification.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn return_address(&self) -> String {
        self.return_address.clone()
    }
}

#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct TypingTable {
    entries: VecDeque<(Conversation, Typer)>,
    deadlines: VecDeque<Deadline>,
}

#[derive(Clone, Debug)]
struct Deadline {
    conversation: Conversation,
    nick: Nick,
    actor: String,
    status: Option<Prefix>,
    return_address: String,
    at: Instant,
}

struct TypingAddress {
    status: Option<Prefix>,
    return_address: String,
}

impl TypingTable {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: VecDeque::new(),
            deadlines: VecDeque::new(),
        })
    }

    #[doc(hidden)]
    pub fn get<'a>(
        &'a self,
        conversation: &Conversation,
    ) -> impl Iterator<Item = &'a Typer> + use<'a> {
        let conversation = conversation.clone();
        self.entries
            .iter()
            .filter(move |(candidate, _)| candidate == &conversation)
            .map(|(_, typer)| typer)
    }

    pub(crate) fn same_entries(&self, other: &Self) -> bool {
        self.entries == other.entries
    }

    fn notify(
        table: &mut Arc<Self>,
        now: Instant,
        conversation: &Conversation,
        nick: Nick,
        actor: String,
        address: TypingAddress,
        state: Typing,
    ) -> Vec<Deadline> {
        let TypingAddress {
            status,
            return_address,
        } = address;
        let table = Arc::make_mut(table);
        let mut evicted = Vec::new();
        let entry = table
            .entries
            .iter()
            .position(|(candidate, typer)| candidate == conversation && typer.nick == nick);
        let deadline = table
            .deadlines
            .iter()
            .position(|item| &item.conversation == conversation && item.nick == nick);
        if state == Typing::Done {
            entry.and_then(|position| table.entries.remove(position));
            deadline.and_then(|position| table.deadlines.remove(position));
        } else {
            let typer = Typer {
                nick: nick.clone(),
                conversation: conversation.clone(),
                state,
                status,
                return_address: return_address.clone(),
            };
            if let Some(position) = entry {
                if table.entries[position].1.state != state
                    || table.entries[position].1.status != status
                    || table.entries[position].1.return_address != return_address
                {
                    table.entries[position].1 = typer;
                }
            } else {
                table.entries.push_back((conversation.clone(), typer));
            }
            let value = Deadline {
                conversation: conversation.clone(),
                nick,
                actor,
                status,
                return_address,
                at: now + Duration::from_secs(if state == Typing::Active { 6 } else { 30 }),
            };
            if let Some(position) = deadline {
                table.deadlines[position] = value;
            } else {
                table.deadlines.push_back(value);
            }
            if table
                .entries
                .iter()
                .filter(|(candidate, _)| candidate == conversation)
                .count()
                > MAX_TYPERS_PER_CONVERSATION
                && let Some(position) = table
                    .entries
                    .iter()
                    .position(|(candidate, _)| candidate == conversation)
            {
                table.entries.remove(position);
                evicted.extend(table.deadlines.remove(position));
            }
            if table.entries.len() > MAX_TYPERS {
                table.entries.pop_front();
                evicted.extend(table.deadlines.pop_front());
            }
        }
        evicted
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.deadlines.iter().map(|deadline| deadline.at).min()
    }

    fn expire(table: &mut Arc<Self>, now: Instant) -> Vec<TypingChanged> {
        let table = Arc::make_mut(table);
        let mut expired = Vec::new();
        table.deadlines.retain(|deadline| {
            if now >= deadline.at {
                expired.push(deadline.clone());
                false
            } else {
                true
            }
        });
        for deadline in &expired {
            table.entries.retain(|(conversation, typer)| {
                conversation != &deadline.conversation || typer.nick != deadline.nick
            });
        }
        expired
            .into_iter()
            .map(|deadline| stopped(deadline, TypingReason::TimedOut))
            .collect()
    }

    fn stop_sender(
        table: &mut Arc<Self>,
        conversation: &Conversation,
        nick: &Nick,
    ) -> Option<Deadline> {
        Self::stop_where(table, |candidate, typer| {
            candidate == conversation && typer == nick
        })
        .pop()
    }

    fn stop_where(
        table: &mut Arc<Self>,
        predicate: impl Fn(&Conversation, &Nick) -> bool,
    ) -> Vec<Deadline> {
        let table = Arc::make_mut(table);
        table
            .entries
            .retain(|(conversation, typer)| !predicate(conversation, &typer.nick));
        let mut removed = Vec::new();
        table.deadlines.retain(|deadline| {
            if predicate(&deadline.conversation, &deadline.nick) {
                removed.push(deadline.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    fn rename(table: &mut Arc<Self>, old: &Nick, new: &Nick) {
        let table = Arc::make_mut(table);
        for (_, typer) in &mut table.entries {
            if &typer.nick == old {
                typer.nick = new.clone();
            }
        }
        for deadline in &mut table.deadlines {
            if &deadline.nick == old {
                deadline.nick = new.clone();
                let suffix = deadline
                    .actor
                    .find(['!', '@'])
                    .map_or("", |index| &deadline.actor[index..]);
                deadline.actor = format!("{new}{suffix}");
            }
        }
        table.deduplicate();
    }

    pub(crate) fn refold(table: &mut Arc<Self>, mapping: crate::Names) {
        let table = Arc::make_mut(table);
        for (conversation, typer) in &mut table.entries {
            *conversation = refold_conversation(conversation, mapping);
            typer.nick = mapping.nick(typer.nick.as_ref());
        }
        for deadline in &mut table.deadlines {
            deadline.conversation = refold_conversation(&deadline.conversation, mapping);
            deadline.nick = mapping.nick(deadline.nick.as_ref());
        }
        table.deduplicate();
    }

    fn deduplicate(&mut self) {
        let mut seen = Vec::new();
        self.entries.retain(|(conversation, typer)| {
            let key = (conversation.clone(), typer.nick.clone());
            !seen.contains(&key) && {
                seen.push(key);
                true
            }
        });
        let mut seen = Vec::new();
        self.deadlines.retain(|deadline| {
            let key = (deadline.conversation.clone(), deadline.nick.clone());
            !seen.contains(&key) && {
                seen.push(key);
                true
            }
        });
    }
}

impl PartialEq for TypingTable {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl Eq for TypingTable {}

impl Session {
    pub(crate) fn report_typing(
        &mut self,
        now: Instant,
        message: &LineRef<'_>,
        is_batched: bool,
    ) -> bool {
        if is_batched || !message.command_is("TAGMSG") {
            return false;
        }
        let state = match message.tag("+typing").as_deref() {
            Some("active") => Typing::Active,
            Some("paused") => Typing::Paused,
            Some("done") => Typing::Done,
            _ => return false,
        };
        let (Some(Source::User { nick, .. }), Some(actor)) =
            (message.source(), message.raw_source())
        else {
            return false;
        };
        let Ok((status, LineTarget::Conversation(conversation))) =
            classify_line_with_names(message, self.nick(), self.isupport.value(), self.names())
        else {
            return false;
        };
        let return_address = match &conversation {
            Conversation::Channel(_) => message.param(0).unwrap_or(conversation.as_str()),
            Conversation::Private(_) => conversation.as_str(),
        }
        .to_owned();
        let authorship = self.authorship(message);
        if authorship == Authorship::Other {
            let nick = self.names().nick(nick);
            let evicted = TypingTable::notify(
                &mut self.state.typing,
                now,
                &conversation,
                nick,
                actor.to_owned(),
                TypingAddress {
                    status,
                    return_address: return_address.clone(),
                },
                state,
            );
            for deadline in evicted {
                self.outbox.event(Event::TypingChanged(stopped(
                    deadline,
                    TypingReason::TimedOut,
                )));
            }
        }
        self.outbox.event(Event::TypingChanged(TypingChanged {
            actor: actor.to_owned(),
            authorship,
            conversation,
            status,
            return_address,
            state,
            reason: TypingReason::Notified,
        }));
        true
    }

    pub(crate) fn handle_typing_timeout(&mut self, now: Instant) {
        let before = Arc::clone(&self.state.typing);
        for change in TypingTable::expire(&mut self.state.typing, now) {
            self.outbox.event(Event::TypingChanged(change));
        }
        if !before.same_entries(&self.state.typing) {
            self.typing_generation = self.typing_generation.wrapping_add(1);
        }
    }

    pub(crate) fn stop_typing_for_message(&mut self, message: &LineRef<'_>) {
        if !matches!(message.command(), Command::Privmsg | Command::Notice) {
            return;
        }
        let Some(Source::User { nick, .. }) = message.source() else {
            return;
        };
        let Ok((_, LineTarget::Conversation(conversation))) =
            classify_line_with_names(message, self.nick(), self.isupport.value(), self.names())
        else {
            return;
        };
        let nick = self.names().nick(nick);
        if let Some(deadline) =
            TypingTable::stop_sender(&mut self.state.typing, &conversation, &nick)
        {
            self.outbox.event(Event::TypingChanged(stopped(
                deadline,
                TypingReason::Messaged,
            )));
        }
    }

    pub(crate) fn stop_typing_for_departure(&mut self, message: &LineRef<'_>) {
        let mapping = self.names();
        let (removed, silent) = match message.command() {
            Command::Part => {
                let (Some(Source::User { nick, .. }), Some(channels)) =
                    (message.source(), message.param(0))
                else {
                    return;
                };
                let own = self.authorship(message) == Authorship::Own;
                let nick = mapping.nick(nick);
                let mut removed = Vec::new();
                for channel in channels.split(',') {
                    let conversation = Conversation::Channel(mapping.channel(channel));
                    if own {
                        removed.extend(TypingTable::stop_where(
                            &mut self.state.typing,
                            |candidate, _| candidate == &conversation,
                        ));
                    } else {
                        removed.extend(TypingTable::stop_sender(
                            &mut self.state.typing,
                            &conversation,
                            &nick,
                        ));
                    }
                }
                (removed, own)
            }
            Command::Kick => {
                let (Some(channel), Some(nick)) = (message.param(0), message.param(1)) else {
                    return;
                };
                let conversation = Conversation::Channel(mapping.channel(channel));
                let own = crate::names_equal(mapping, nick, self.nick());
                let nick = mapping.nick(nick);
                (
                    TypingTable::stop_where(&mut self.state.typing, |candidate, typer| {
                        candidate == &conversation && (own || typer == &nick)
                    }),
                    own,
                )
            }
            Command::Quit => {
                let Some(Source::User { nick, .. }) = message.source() else {
                    return;
                };
                let own = self.authorship(message) == Authorship::Own;
                let nick = mapping.nick(nick);
                (
                    TypingTable::stop_where(&mut self.state.typing, |_, typer| {
                        own || typer == &nick
                    }),
                    own,
                )
            }
            _ => return,
        };
        if !silent {
            for deadline in removed {
                self.outbox.event(Event::TypingChanged(stopped(
                    deadline,
                    TypingReason::Departed,
                )));
            }
        }
    }

    pub(crate) fn carry_typing_for_nick(&mut self, message: &LineRef<'_>) {
        let (Some(Source::User { nick, .. }), Some(new)) = (message.source(), message.param(0))
        else {
            return;
        };
        if message.command() != Command::Nick {
            return;
        }
        let mapping = self.names();
        TypingTable::rename(
            &mut self.state.typing,
            &mapping.nick(nick),
            &mapping.nick(new),
        );
    }
}

fn refold_conversation(conversation: &Conversation, mapping: crate::Names) -> Conversation {
    match conversation {
        Conversation::Channel(channel) => Conversation::Channel(mapping.channel(channel.as_ref())),
        Conversation::Private(nick) => Conversation::Private(mapping.nick(nick.as_ref())),
    }
}

fn stopped(deadline: Deadline, reason: TypingReason) -> TypingChanged {
    TypingChanged {
        actor: deadline.actor,
        authorship: Authorship::Other,
        conversation: deadline.conversation,
        status: deadline.status,
        return_address: deadline.return_address,
        state: Typing::Done,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use papo_proto::{Casemapping, Typing};

    use super::{Typer, TypingTable};
    use crate::Conversation;

    #[test]
    fn conversation_view_preserves_start_order() {
        let mapping = Casemapping::Rfc1459;
        let conversation = Conversation::Channel(mapping.channel("#papo"));
        let other = Conversation::Channel(mapping.channel("#rust"));
        let mut table = TypingTable::new();
        Arc::make_mut(&mut table).entries.extend([
            (
                conversation.clone(),
                Typer {
                    nick: mapping.nick("Alice"),
                    conversation: conversation.clone(),
                    state: Typing::Active,
                    status: None,
                    return_address: "#papo".into(),
                },
            ),
            (
                other.clone(),
                Typer {
                    nick: mapping.nick("bob"),
                    conversation: other.clone(),
                    state: Typing::Paused,
                    status: None,
                    return_address: "#rust".into(),
                },
            ),
            (
                conversation.clone(),
                Typer {
                    nick: mapping.nick("carol"),
                    conversation: conversation.clone(),
                    state: Typing::Paused,
                    status: Some(papo_proto::Prefix::__new('o', '@')),
                    return_address: "@#papo".into(),
                },
            ),
        ]);

        let nicks: Vec<_> = table.get(&conversation).map(|typer| &typer.nick).collect();
        assert_eq!(nicks, ["Alice", "carol"]);
    }
}
