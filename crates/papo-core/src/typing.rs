use std::{collections::VecDeque, sync::Arc};

use papo_proto::{LineRef, Nick, Source, Typing};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TypingTable {
    entries: VecDeque<(Conversation, Typer)>,
}

impl TypingTable {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: VecDeque::new(),
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
        conversation: Conversation,
        nick: Nick,
        status: Option<Vec<char>>,
        state: Typing,
    ) {
        let table = Arc::make_mut(table);
        let position = table
            .entries
            .iter()
            .position(|(candidate, typer)| candidate == &conversation && typer.nick == nick);
        if state == Typing::Done {
            if let Some(position) = position {
                table.entries.remove(position);
            }
        } else if let Some(position) = position {
            table.entries[position].1.state = state;
            table.entries[position].1.status = status;
        } else {
            table.entries.push_back((
                conversation,
                Typer {
                    nick,
                    state,
                    status,
                },
            ));
        }
    }
}

impl Session {
    pub(crate) fn report_typing(&mut self, message: &LineRef<'_>, is_batched: bool) -> bool {
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
                conversation.clone(),
                self.isupport.value().casemapping().nick(nick),
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
