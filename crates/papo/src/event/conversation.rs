use crate::Typing;

use super::Source;

/// A reaction was added to or removed from a message.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Reacted {
    /// The user or server that changed the reaction.
    pub source: Option<Source>,
    /// The channel or nickname containing the message.
    pub target: String,
    /// The reaction text.
    pub reaction: String,
    /// The identifier of the message being reacted to.
    pub msgid: String,
    /// Whether the reaction was removed.
    pub removed: bool,
}

/// A user's typing state changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TypingChanged {
    /// The user or server that sent the notification.
    pub source: Option<Source>,
    /// The channel or nickname where typing is occurring.
    pub target: String,
    /// The new typing state.
    pub state: Typing,
}
