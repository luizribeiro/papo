use std::{collections::VecDeque, sync::Arc};

use papo_proto::{Nick, Typing};

use crate::Conversation;

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
