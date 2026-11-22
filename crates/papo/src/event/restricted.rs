use std::time::SystemTime;

use super::{
    CtcpQuery, CtcpReply, Incomplete, Message, MessageId, Notice, NoticeTarget, Other, Reacted,
    Redacted, ServerNotice, ServerTime, TagMsg, TypingChanged,
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

macro_rules! line_types {
    ($($(#[$doc:meta])* $variant:ident($ty:ty)),+ $(,)?) => {
        /// The payload of a [`Restricted`] line.
        #[derive(Clone, Debug, Eq, PartialEq)]
        #[non_exhaustive]
        pub enum RestrictedLine {
            $($(#[$doc])* $variant($ty),)+
        }

        /// A line payload borrowed from a plain arm or from [`Restricted::line`], so each
        /// accessor is written once for both.
        #[derive(Clone, Copy)]
        pub(crate) enum LineView<'a> {
            $($variant(&'a $ty),)+
        }

        impl RestrictedLine {
            const fn view(&self) -> LineView<'_> {
                match self {
                    $(Self::$variant(line) => LineView::$variant(line),)+
                }
            }
        }

        impl crate::Event {
            pub(crate) const fn line_view(&self) -> Option<LineView<'_>> {
                match self {
                    $(Self::$variant(line) => Some(LineView::$variant(line)),)+
                    Self::Restricted(restricted) => Some(restricted.line.view()),
                    _ => None,
                }
            }
        }

        /// Moves a status-prefixed line into [`crate::Event::Restricted`], so that no plain arm
        /// ever carries one. The event stream and every history item both end here.
        pub(crate) fn separate(
            event: crate::Event,
            own_nick: &str,
            isupport: &papo_proto::Isupport,
        ) -> crate::Event {
            let (audience, line) = match event {
                $(crate::Event::$variant(mut line) => match line.take_status(own_nick, isupport) {
                    Some(audience) => (audience, RestrictedLine::$variant(line)),
                    None => return crate::Event::$variant(line),
                },)+
                event => return event,
            };
            crate::Event::Restricted(Restricted { audience, line })
        }
    };
}

line_types! {
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
    /// A line papo did not turn into a typed payload.
    Other(Other),
}

trait TakeStatus {
    fn take_status(&mut self, own_nick: &str, isupport: &papo_proto::Isupport) -> Option<Prefix>;
}

macro_rules! status_field {
    ($($ty:ty),+ $(,)?) => {
        $(impl TakeStatus for $ty {
            fn take_status(&mut self, _: &str, _: &papo_proto::Isupport) -> Option<Prefix> {
                self.status.take()
            }
        })+
    };
}

status_field!(
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

impl TakeStatus for Incomplete {
    fn take_status(&mut self, own_nick: &str, isupport: &papo_proto::Isupport) -> Option<Prefix> {
        match self {
            Self::Message(message) => message.take_status(own_nick, isupport),
            Self::Notice(notice) => notice.take_status(own_nick, isupport),
        }
    }
}

// An untyped line was never classified, so its target is read here: a status sign on it names
// the same audience a typed line would carry.
impl TakeStatus for Other {
    fn take_status(&mut self, own_nick: &str, isupport: &papo_proto::Isupport) -> Option<Prefix> {
        papo_core::classify_conversation(&self.line.as_ref(), own_nick, isupport)
            .ok()
            .and_then(|(status, _)| status)
    }
}

impl<'a> LineView<'a> {
    pub(crate) fn conversation(self) -> Option<Conversation> {
        match self {
            Self::Message(line) => Some(line.conversation.clone()),
            Self::Notice(line) => Some(line.conversation().clone()),
            Self::Incomplete(line) => Some(line.conversation().clone()),
            Self::ServerNotice(line) => match &line.to {
                NoticeTarget::Channel(channel) => Some(Conversation::Channel(channel.clone())),
                NoticeTarget::Me | NoticeTarget::Broadcast(_) => None,
            },
            Self::CtcpQuery(line) => Some(line.conversation.clone()),
            Self::CtcpReply(line) => Some(line.conversation.clone()),
            Self::TagMsg(line) => Some(line.conversation.clone()),
            Self::Reacted(line) => Some(line.conversation.clone()),
            Self::Redacted(line) => Some(line.conversation.clone()),
            Self::TypingChanged(line) => Some(line.conversation().clone()),
            Self::Other(_) => None,
        }
    }

    pub(crate) const fn msgid(self) -> Option<&'a MessageId> {
        match self {
            Self::Message(line) | Self::Incomplete(Incomplete::Message(line)) => {
                line.msgid.as_ref()
            }
            Self::Notice(line) | Self::Incomplete(Incomplete::Notice(line)) => line.msgid(),
            Self::ServerNotice(line) => line.msgid(),
            Self::CtcpQuery(line) => line.msgid.as_ref(),
            Self::CtcpReply(line) => line.msgid.as_ref(),
            Self::TagMsg(line) => line.msgid.as_ref(),
            Self::Reacted(line) => line.msgid.as_ref(),
            Self::Redacted(line) => line.msgid.as_ref(),
            Self::Other(line) => line.msgid.as_ref(),
            Self::TypingChanged(_) => None,
        }
    }

    pub(crate) fn server_time(self) -> Option<ServerTime> {
        match self {
            Self::Message(line) | Self::Incomplete(Incomplete::Message(line)) => line.server_time,
            Self::Notice(line) | Self::Incomplete(Incomplete::Notice(line)) => line.server_time,
            Self::ServerNotice(line) => line.server_time,
            Self::CtcpQuery(line) => line.server_time,
            Self::CtcpReply(line) => line.server_time,
            Self::TagMsg(line) => super::tagged_from_tags(&line.tags),
            Self::Reacted(line) => line.server_time,
            Self::Redacted(line) => line.server_time,
            Self::TypingChanged(line) => line.server_time,
            Self::Other(line) => line.server_time,
        }
    }

    pub(crate) const fn received_at(self) -> SystemTime {
        match self {
            Self::Message(line) | Self::Incomplete(Incomplete::Message(line)) => line.received_at,
            Self::Notice(line) | Self::Incomplete(Incomplete::Notice(line)) => line.received_at,
            Self::ServerNotice(line) => line.received_at,
            Self::CtcpQuery(line) => line.received_at,
            Self::CtcpReply(line) => line.received_at,
            Self::TagMsg(line) => line.received_at,
            Self::Reacted(line) => line.received_at,
            Self::Redacted(line) => line.received_at,
            Self::TypingChanged(line) => line.received_at,
            Self::Other(line) => line.received_at,
        }
    }
}
