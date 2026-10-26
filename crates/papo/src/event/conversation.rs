use std::time::SystemTime;

use crate::{Conversation, Mask, MessageId, Typing};

use super::{Authorship, Source, Tags, TypingReason, UserSource};

/// A server-wide audience addressed by `*` or a server mask.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Broadcast {
    /// Every eligible client.
    Everyone,
    /// Clients selected by a server mask.
    Mask(Mask),
}

/// A Client-to-Client Protocol query sent to a server-wide audience.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BroadcastCtcp {
    /// The user or server that sent the CTCP message.
    pub actor: UserSource,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// The server-wide audience addressed by the message.
    pub to: Broadcast,
    /// The command with its original spelling.
    pub command: String,
    /// The command parameters, when present.
    pub params: Option<String>,
    /// The server-assigned message identifier, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<SystemTime>,
    /// Time at which the line was received.
    pub received_at: SystemTime,
    /// All message tags in wire order.
    pub tags: Tags,
}

/// A tag-only message sent to a server-wide audience.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BroadcastTagMsg {
    /// The user or server that sent the message.
    pub actor: UserSource,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// The server-wide audience addressed by the message.
    pub to: Broadcast,
    /// Tags in wire order with last-wins lookup.
    pub tags: Tags,
}

fn broadcast_origin(to: &Broadcast, actor: &UserSource) -> crate::Origin {
    match to {
        Broadcast::Mask(mask) => mask.origin(),
        Broadcast::Everyone => actor.nick().origin(),
    }
}

macro_rules! broadcast_origin {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl $ty {
                /// Which `Client` produced this value (not the sender or the server).
                #[must_use]
                pub fn origin(&self) -> crate::Origin {
                    broadcast_origin(&self.to, &self.actor)
                }
            }
        )+
    };
}

broadcast_origin!(BroadcastCtcp, BroadcastTagMsg);

/// Whether a reaction was added to or removed from a message.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ReactionAction {
    /// The reaction was added.
    Added,
    /// The reaction was removed.
    Removed,
}

/// A reaction was added to or removed from a message.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Reacted {
    /// The server-assigned identifier of the reaction line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<SystemTime>,
    /// Time at which the line was received.
    pub received_at: SystemTime,
    /// The user that changed the reaction.
    pub actor: UserSource,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// The IRC conversation containing the message.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub conversation: Conversation,
    /// `STATUSMSG` prefixes on the wire address, in wire order.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub status: Option<Vec<char>>,
    /// The message being reacted to.
    pub reacts_to: MessageId,
    /// The reaction text, usually an emoji.
    ///
    /// Compare it exactly; papo performs no case folding or Unicode normalization.
    pub reaction: String,
    /// Whether the reaction was added or removed.
    pub action: ReactionAction,
}

/// A message was removed from history.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Redacted {
    /// The server-assigned identifier of the redaction line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<SystemTime>,
    /// Time at which the line was received.
    pub received_at: SystemTime,
    /// The user or server that removed the message.
    pub actor: Source,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// The IRC conversation containing the message.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub conversation: Conversation,
    /// `STATUSMSG` prefixes on the wire address, in wire order.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub status: Option<Vec<char>>,
    /// The message removed from history.
    pub redacts: MessageId,
    /// The optional human-readable reason.
    pub reason: Option<String>,
}

/// A user's typing state changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TypingChanged {
    /// The user or server that sent the notification.
    pub actor: UserSource,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// The IRC conversation where typing is occurring.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub conversation: Conversation,
    /// `STATUSMSG` prefixes on the wire address, in wire order.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub status: Option<Vec<char>>,
    /// The new typing state.
    pub state: Typing,
    /// Why the state changed.
    pub reason: TypingReason,
}

/// A tag-only message with no more specific typed event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TagMsg {
    /// The user or server that sent the message.
    pub actor: UserSource,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// The IRC conversation addressed by the message.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub conversation: Conversation,
    /// `STATUSMSG` prefixes on the wire address, in wire order.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub status: Option<Vec<char>>,
    /// Tags in wire order with last-wins lookup.
    pub tags: Tags,
}

impl Reacted {
    /// Returns the address that reaches the recipients of this line.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn return_address(&self) -> crate::MessageRef {
        crate::message_ref::from_reacted(self)
    }
}

impl Redacted {
    /// Returns the address that reaches the recipients of this line.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn return_address(&self) -> crate::MessageRef {
        crate::message_ref::from_redacted(self)
    }
}

impl TypingChanged {
    /// Returns the address that reaches the recipients of this line.
    ///
    /// A typing notification has no message identifier, so reactions and redactions using this
    /// reference fail with [`crate::InvalidInput::UnusableMessageReference`]. See
    /// [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn return_address(&self) -> crate::MessageRef {
        crate::message_ref::from_typing(self)
    }
}

impl TagMsg {
    /// Returns the address that reaches the recipients of this line.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn return_address(&self) -> crate::MessageRef {
        crate::MessageRef::from(self)
    }
}

macro_rules! origin {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl $ty {
                /// Which `Client` produced this value (not the sender or the server).
                #[must_use]
                pub fn origin(&self) -> crate::Origin {
                    crate::message_ref::conversation_origin(&self.conversation)
                }
            }
        )+
    };
}

origin!(Reacted, Redacted, TypingChanged, TagMsg, ReadMarkerChanged);

/// A conversation's read marker changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ReadMarkerChanged {
    /// Conversation to which the marker applies.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub conversation: Conversation,
    /// The marker time, or `None` when the server reports `*`.
    pub read_until: Option<SystemTime>,
}

/// Severity of an `IRCv3` standard reply.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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

#[cfg(test)]
mod tests {
    use super::{Authorship, Broadcast, BroadcastCtcp, BroadcastTagMsg, Tags};

    #[test]
    fn broadcast_payloads_report_the_client_origin() {
        let origin = crate::Origin::__new(73);
        let mapping = crate::Casemapping::default();
        let actor = super::super::source::user_from_line(
            &papo_proto::Line::new("TAGMSG")
                .param("*")
                .with_source("oper!staff@staff.example")
                .as_ref(),
            mapping,
            origin,
            false,
            false,
        )
        .unwrap();
        for to in [
            Broadcast::Everyone,
            Broadcast::Mask(origin.__mask(mapping, "$*.example.org")),
        ] {
            let event = BroadcastTagMsg {
                actor: actor.clone(),
                authorship: Authorship::Other,
                to,
                tags: Tags::default(),
            };
            assert_eq!(event.origin(), origin);
        }
        let ctcp = BroadcastCtcp {
            actor,
            authorship: Authorship::Other,
            to: Broadcast::Mask(origin.__mask(mapping, "$*.example.org")),
            command: "VERSION".into(),
            params: None,
            msgid: None,
            server_time: None,
            received_at: std::time::UNIX_EPOCH,
            tags: Tags::default(),
        };
        assert_eq!(ctcp.origin(), origin);
    }
}
