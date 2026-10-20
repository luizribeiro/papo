use std::{collections::VecDeque, sync::Arc, time::Duration};

use papo_proto::{Command, LineRef, Nick, Source, Typing};
use web_time::Instant;

use crate::{Authorship, Conversation, Event, Session, conversation::classify_line};

/// Why a user's typing state changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    pub conversation: Conversation,
    /// `STATUSMSG` prefixes on the typing notification's wire address.
    pub status: Option<Vec<char>>,
    /// The new typing state.
    pub state: Typing,
    /// Why the state changed.
    pub reason: TypingReason,
}

/// A user currently composing a message in a conversation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Typer {
    /// The user's nickname.
    pub nick: Nick,
    /// Whether the user is actively typing or has paused.
    pub state: Typing,
    /// `STATUSMSG` prefixes on the typing notification's wire address.
    pub status: Option<Vec<char>>,
}

#[derive(Clone, Debug)]
pub(crate) struct TypingTable {
    entries: VecDeque<(Conversation, Typer)>,
    deadlines: VecDeque<Deadline>,
}

#[derive(Clone, Debug)]
struct Deadline {
    conversation: Conversation,
    nick: Nick,
    actor: String,
    status: Option<Vec<char>>,
    at: Instant,
}

impl TypingTable {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: VecDeque::new(),
            deadlines: VecDeque::new(),
        })
    }

    pub(crate) fn get<'a>(
        &'a self,
        conversation: &'a Conversation,
    ) -> impl Iterator<Item = &'a Typer> {
        self.entries
            .iter()
            .filter(move |(candidate, _)| candidate == conversation)
            .map(|(_, typer)| typer)
    }

    fn notify(
        table: &mut Arc<Self>,
        now: Instant,
        conversation: Conversation,
        nick: Nick,
        actor: String,
        status: Option<Vec<char>>,
        state: Typing,
    ) {
        let table = Arc::make_mut(table);
        let entry = table
            .entries
            .iter()
            .position(|(candidate, typer)| candidate == &conversation && typer.nick == nick);
        let deadline = table
            .deadlines
            .iter()
            .position(|item| item.conversation == conversation && item.nick == nick);
        if state == Typing::Done {
            entry.and_then(|position| table.entries.remove(position));
            deadline.and_then(|position| table.deadlines.remove(position));
        } else {
            let typer = Typer {
                nick: nick.clone(),
                state,
                status: status.clone(),
            };
            if let Some(position) = entry {
                table.entries[position].1 = typer;
            } else {
                table.entries.push_back((conversation.clone(), typer));
            }
            let value = Deadline {
                conversation,
                nick,
                actor,
                status,
                at: now + Duration::from_secs(if state == Typing::Active { 6 } else { 30 }),
            };
            if let Some(position) = deadline {
                table.deadlines[position] = value;
            } else {
                table.deadlines.push_back(value);
            }
        }
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
        let table = Arc::make_mut(table);
        table
            .entries
            .retain(|(candidate, typer)| candidate != conversation || &typer.nick != nick);
        let position = table
            .deadlines
            .iter()
            .position(|item| &item.conversation == conversation && &item.nick == nick)?;
        table.deadlines.remove(position)
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
        let Ok((status, conversation)) = classify_line(message, self.nick(), self.isupport.value())
        else {
            return false;
        };
        let authorship = self.authorship(message);
        if authorship == Authorship::Other {
            TypingTable::notify(
                &mut self.state.typing,
                now,
                conversation.clone(),
                self.isupport.value().casemapping().nick(nick),
                actor.to_owned(),
                status.clone(),
                state,
            );
        }
        self.outbox.event(Event::TypingChanged(TypingChanged {
            actor: actor.to_owned(),
            authorship,
            conversation,
            status,
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
        if before != self.state.typing {
            self.state_generation = self.state_generation.wrapping_add(1);
        }
    }

    pub(crate) fn stop_typing_for_message(&mut self, message: &LineRef<'_>) {
        if !matches!(message.command(), Command::Privmsg | Command::Notice) {
            return;
        }
        let Some(Source::User { nick, .. }) = message.source() else {
            return;
        };
        let Ok((_, conversation)) = classify_line(message, self.nick(), self.isupport.value())
        else {
            return;
        };
        let nick = self.isupport.value().casemapping().nick(nick);
        if let Some(deadline) =
            TypingTable::stop_sender(&mut self.state.typing, &conversation, &nick)
        {
            self.outbox.event(Event::TypingChanged(stopped(
                deadline,
                TypingReason::Messaged,
            )));
        }
    }
}

fn stopped(deadline: Deadline, reason: TypingReason) -> TypingChanged {
    TypingChanged {
        actor: deadline.actor,
        authorship: Authorship::Other,
        conversation: deadline.conversation,
        status: deadline.status,
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
                    state: Typing::Active,
                    status: None,
                },
            ),
            (
                other,
                Typer {
                    nick: mapping.nick("bob"),
                    state: Typing::Paused,
                    status: None,
                },
            ),
            (
                conversation.clone(),
                Typer {
                    nick: mapping.nick("carol"),
                    state: Typing::Paused,
                    status: Some(vec!['@']),
                },
            ),
        ]);

        let nicks: Vec<_> = table.get(&conversation).map(|typer| &typer.nick).collect();
        assert_eq!(nicks, ["Alice", "carol"]);
    }
}
