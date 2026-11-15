use super::{
    Audience, CtcpQuery, CtcpReply, Incomplete, Message, Notice, Reacted, Redacted, ServerNotice,
    TagMsg, TypingChanged,
};
use crate::{Conversation, Prefix};

/// A line that reached only the members of a channel holding [`Self::audience`] or a higher rank.
///
/// On the wire it is an ordinary line whose target carries a `STATUSMSG` sign, such as
/// `PRIVMSG @#ops`. papo delivers it only here, never in [`crate::Event::Message`] or another
/// plain arm, so code that does not match this arm never sees an ops-only line and cannot answer
/// one to the whole channel. See [`Conversation`] for status-prefixed line guidance.
///
/// ```
/// # async fn handle(client: &papo::Client, event: papo::Event) -> Result<(), papo::Error> {
/// if let papo::Event::Restricted(restricted) = event
///     && let papo::RestrictedLine::Message(message) = &restricted.line
/// {
///     client.reply(message, "on it").await?;
/// }
/// # Ok(()) }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Restricted {
    /// The lowest-ranked `STATUSMSG` prefix on the wire address.
    pub audience: Prefix,
    /// The line, as the payload its plain arm would carry.
    ///
    /// Answering it with [`crate::Client::reply`], [`crate::Client::react`],
    /// [`crate::Client::redact`] or its `return_address()` keeps the restricted address.
    pub line: RestrictedLine,
}

/// The payload of a [`Restricted`] line.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RestrictedLine {
    /// A `PRIVMSG` or `ACTION`.
    Message(Message),
    /// A `NOTICE` from a user.
    Notice(Notice),
    /// A message or notice whose `draft/multiline` batch ended before its closing line.
    Incomplete(Incomplete),
    /// A server notice to the channel.
    ServerNotice(ServerNotice),
    /// A Client-to-Client Protocol query.
    CtcpQuery(CtcpQuery),
    /// A Client-to-Client Protocol reply.
    CtcpReply(CtcpReply),
    /// A tag-only message with no more specific typed payload.
    TagMsg(TagMsg),
    /// A tag-only reaction or unreaction.
    Reacted(Reacted),
    /// A redaction.
    Redacted(Redacted),
    /// A user's typing state changed.
    TypingChanged(TypingChanged),
}

variant_accessors! { RestrictedLine;
    (Message, as_message, Message),
    (Notice, as_notice, Notice),
    (Incomplete, as_incomplete, Incomplete),
    (ServerNotice, as_server_notice, ServerNotice),
    (CtcpQuery, as_ctcp_query, CtcpQuery),
    (CtcpReply, as_ctcp_reply, CtcpReply),
    (TagMsg, as_tag_msg, TagMsg),
    (Reacted, as_reacted, Reacted),
    (Redacted, as_redacted, Redacted),
    (TypingChanged, as_typing_changed, TypingChanged),
}

impl RestrictedLine {
    pub(crate) fn conversation(&self) -> Option<Conversation> {
        match self {
            Self::Message(line) => Some(line.conversation.clone()),
            Self::Notice(line) => Some(line.conversation.clone()),
            Self::Incomplete(line) => Some(line.conversation().clone()),
            Self::ServerNotice(line) => match &line.to {
                Audience::Channel(channel) => Some(Conversation::Channel(channel.clone())),
                Audience::Me | Audience::Broadcast(_) => None,
            },
            Self::CtcpQuery(line) => Some(line.conversation.clone()),
            Self::CtcpReply(line) => Some(line.conversation.clone()),
            Self::TagMsg(line) => Some(line.conversation.clone()),
            Self::Reacted(line) => Some(line.conversation.clone()),
            Self::Redacted(line) => Some(line.conversation.clone()),
            Self::TypingChanged(line) => Some(line.conversation.clone()),
        }
    }
}

macro_rules! separate_restricted {
    ($($variant:ident),+ $(,)?) => {
        /// Moves a status-prefixed line into [`crate::Event::Restricted`], so that no plain arm
        /// ever carries one. Every event an application receives passes through here.
        pub(crate) fn separate(event: crate::Event) -> crate::Event {
            let (audience, line) = match event {
                $(crate::Event::$variant(mut line) => match line.status.take() {
                    Some(audience) => (audience, RestrictedLine::$variant(line)),
                    None => return crate::Event::$variant(line),
                },)+
                crate::Event::Incomplete(mut line) => {
                    let status = match &mut line {
                        Incomplete::Message(message) => message.status.take(),
                        Incomplete::Notice(notice) => notice.status.take(),
                    };
                    match status {
                        Some(audience) => (audience, RestrictedLine::Incomplete(line)),
                        None => return crate::Event::Incomplete(line),
                    }
                }
                event => return event,
            };
            crate::Event::Restricted(Restricted { audience, line })
        }
    };
}

separate_restricted!(
    Message,
    Notice,
    ServerNotice,
    CtcpQuery,
    CtcpReply,
    TagMsg,
    Reacted,
    Redacted,
    TypingChanged,
);
