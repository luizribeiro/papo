use std::collections::vec_deque;

use crate::{Conversation, Nick, Prefix};

/// A user currently composing a message in a conversation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Typer {
    core: papo_core::Typer,
    return_address: crate::MessageRef,
}

impl Typer {
    /// Returns the user's nickname.
    #[must_use]
    pub const fn nick(&self) -> &Nick {
        self.core.nick()
    }

    /// Returns whether the user has paused rather than actively typing.
    #[must_use]
    pub const fn is_paused(&self) -> bool {
        self.core.is_paused()
    }

    /// Returns the lowest-ranked `STATUSMSG` prefix on the typing notification's wire address.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub const fn status(&self) -> Option<Prefix> {
        self.core.status()
    }

    /// Returns the address that reaches the recipients of the typing notification.
    ///
    /// A typing notification has no message identifier, so reactions and redactions using this
    /// reference fail with [`crate::InvalidInput::UnusableMessageReference`]. See
    /// [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn return_address(&self) -> crate::MessageRef {
        self.return_address.clone()
    }

    fn new(core: &papo_core::Typer, state: &papo_core::State) -> Self {
        let conversation = core.__conversation();
        let (recipient, channel) = match conversation {
            Conversation::Private(nick) => {
                let account = state
                    .user(nick)
                    .map_or(papo_core::Account::Unknown, |user| user.account().clone());
                let guard = papo_core::RecipientGuard::__from_parts(
                    nick.clone(),
                    account,
                    state.__reply_token_for(nick),
                );
                (
                    Some(crate::message_ref::RecipientGuard::from_core(&guard)),
                    None,
                )
            }
            Conversation::Channel(channel) => {
                (None, Some(Box::new(state.__channel_guard(channel))))
            }
            _ => unreachable!("papo-core returned an unknown conversation kind"),
        };
        Self {
            core: core.clone(),
            return_address: crate::MessageRef::new(
                core.__return_address().to_owned(),
                Vec::new(),
                crate::message_ref::conversation_origin(conversation),
                None,
                recipient,
                channel,
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TypingTable {
    entries: std::collections::VecDeque<(Conversation, Typer)>,
}

impl TypingTable {
    pub(crate) fn new(table: &papo_core::TypingTable, state: &papo_core::State) -> Self {
        let entries = table
            .__entries()
            .map(|(conversation, typer)| (conversation.clone(), Typer::new(typer, state)))
            .collect();
        Self { entries }
    }

    pub(crate) fn get(&self, conversation: Option<Conversation>) -> Typers<'_> {
        Typers {
            entries: self.entries.iter(),
            conversation,
        }
    }
}

/// An iterator over users typing in one conversation.
pub struct Typers<'a> {
    entries: vec_deque::Iter<'a, (Conversation, Typer)>,
    conversation: Option<Conversation>,
}

impl<'a> Iterator for Typers<'a> {
    type Item = &'a Typer;

    fn next(&mut self) -> Option<Self::Item> {
        let conversation = self.conversation.as_ref()?;
        self.entries
            .find(|(candidate, _)| candidate == conversation)
            .map(|(_, typer)| typer)
    }
}
