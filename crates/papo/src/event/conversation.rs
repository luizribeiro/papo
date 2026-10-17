use std::time::SystemTime;

use crate::{MessageId, Target, Typing};

use super::{Source, Tags};

/// A reaction was added to or removed from a message.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Reacted {
    /// The user or server that changed the reaction.
    pub source: Option<Source>,
    /// The conversation containing the message.
    pub target: Target,
    /// The message being reacted to.
    pub message: MessageId,
    /// The reaction text.
    pub reaction: String,
    /// Whether the reaction was removed.
    pub removed: bool,
}

/// A message was removed from history.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Redacted {
    /// The user or server that removed the message.
    pub source: Option<Source>,
    /// The conversation containing the message.
    pub target: Target,
    /// The message removed from history.
    pub message: MessageId,
    /// The optional human-readable reason.
    pub reason: Option<String>,
}

/// A user's typing state changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TypingChanged {
    /// The user or server that sent the notification.
    pub source: Option<Source>,
    /// The channel or nickname where typing is occurring.
    pub target: Target,
    /// The new typing state.
    pub state: Typing,
}

/// A tag-only message with no more specific typed event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TagMsg {
    /// The message source, when supplied by the server.
    pub source: Option<Source>,
    /// The message target.
    pub target: Target,
    /// Tags in wire order with last-wins lookup.
    pub tags: Tags,
}

/// A target's read marker changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ReadMarker {
    /// The channel or nickname to which the marker applies.
    pub target: Target,
    /// The marker time, or `None` when the server reports `*`.
    pub time: Option<SystemTime>,
}

/// Severity of an `IRCv3` standard reply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StandardReplyKind {
    /// The requested operation failed.
    Fail,
    /// The requested operation completed with a warning.
    Warn,
    /// The server supplied informational context.
    Note,
}

/// An unsolicited `IRCv3` standard reply.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct StandardReply {
    /// Reply severity.
    pub kind: StandardReplyKind,
    /// Case-normalized command associated with the reply.
    pub command: String,
    /// Case-normalized reply code.
    pub code: String,
    /// Additional diagnostic parameters.
    pub context: Vec<String>,
    /// Human-readable reply description.
    pub description: String,
}
