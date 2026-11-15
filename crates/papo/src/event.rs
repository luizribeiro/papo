#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)
)]

use papo_core::{Account, RequestId, Response, Welcome, sts::StsAction};
use std::time::SystemTime;

use papo_proto::{
    Casemapping, ChannelName, Ctcp as ProtoCtcp, Isupport, Line as ProtoLine, LineRef, MessageId,
    Prefix, strip_formatting,
};

use crate::{Client, Conversation, Disconnected, ModeAction, ModeChange, ServerTime};

pub(crate) const fn can_prove_non_bot(message_tags: bool, bot_mode: bool) -> bool {
    message_tags && bot_mode
}

#[path = "event/channel.rs"]
mod channel;
#[path = "event/connection.rs"]
mod connection;
#[path = "event/conversation.rs"]
mod conversation;
#[path = "event/restricted.rs"]
mod restricted;
#[path = "event/server.rs"]
mod server;
#[path = "event/source.rs"]
#[allow(
    clippy::expect_used,
    reason = "owned source lines are constructed only after a source was parsed"
)]
pub(crate) mod source;
#[path = "event/tags.rs"]
mod tags;
#[path = "event/user.rs"]
mod user;

pub use channel::{
    ChannelRenamed, ChannelSynced, Invited, Joined, Kicked, MembersRefreshed, ModeChanged, Parted,
    TopicChanged,
};
pub use connection::{
    CapsChanged, CasemappingChanged, ConnectPhase, Connected, Connecting, FeaturesChanged, Lagged,
    MessageGap, Motd,
};
pub use conversation::{
    Broadcast, BroadcastCtcp, BroadcastTagMsg, Reacted, ReactionAction, ReadMarkerChanged,
    Redacted, StandardReply, StandardReplyKind, TagMsg, TypingChanged,
};
pub use papo_core::{BatchAbandonReason, TypingReason};
use restricted::LineView;
pub(crate) use restricted::separate as separate_restricted;
pub use restricted::{Restricted, RestrictedLine};
#[cfg(feature = "unstable")]
pub use server::BouncerNetworkChanged;
pub use server::{AutoJoinFailed, MetadataChanged, Warning};
pub use source::Source;
pub use tags::{Tags, TagsIter};
pub use user::{
    AccountChanged, AwayChanged, HostChanged, MonitorOffline, MonitorOnline, NickChanged, Quit,
    RealnameChanged, SplitServers,
};

/// The semantic kind of an incoming user message or notice.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum MessageKind {
    /// A regular text message.
    Text,
    /// A CTCP `ACTION`.
    Action,
}

/// The wire command that carried a server notice.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ServerNoticeKind {
    /// A `NOTICE` command.
    Notice,
    /// A `PRIVMSG` command.
    Privmsg,
    /// A `WALLOPS` command.
    Wallops,
}

/// The audience addressed by a server notice.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Audience {
    /// This client.
    Me,
    /// A channel.
    Channel(crate::ChannelHandle),
    /// A server-wide audience.
    Broadcast(Broadcast),
}

/// An incoming Client-to-Client Protocol query carried by `PRIVMSG`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CtcpQuery {
    /// The user that sent the CTCP message.
    pub actor: crate::UserHandle,
    /// Who the actor is relative to you.
    pub actor_party: Party,
    /// The IRC conversation to which the CTCP message belongs.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub(crate) conversation: Conversation,
    /// The lowest-ranked `STATUSMSG` prefix on the wire address, moved to
    /// [`Restricted::audience`] before delivery.
    pub(crate) status: Option<Prefix>,
    /// The command with its original spelling.
    pub command: crate::Command,
    /// The command parameters, when present.
    pub params: Option<String>,
    /// The server-assigned message identifier, when supplied.
    pub(crate) msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    ///
    /// This also accepts ZNC's `znc.in/server-time-iso` tag, which is not part of any `IRCv3`
    /// specification.
    pub server_time: Option<ServerTime>,
    /// Time at which the line was received, from this device's clock.
    pub received_at: SystemTime,
    /// All message tags in wire order.
    pub tags: Tags,
    pub(crate) receipt: crate::message_ref::Receipt,
}

/// An incoming Client-to-Client Protocol reply carried by `NOTICE`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CtcpReply {
    /// The user that sent the CTCP reply.
    pub actor: crate::UserHandle,
    /// Who the actor is relative to you.
    pub actor_party: Party,
    /// The IRC conversation to which the CTCP reply belongs.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub(crate) conversation: Conversation,
    /// The lowest-ranked `STATUSMSG` prefix on the wire address, moved to
    /// [`Restricted::audience`] before delivery.
    pub(crate) status: Option<Prefix>,
    /// The command with its original spelling.
    pub command: crate::Command,
    /// The command parameters, when present.
    pub params: Option<String>,
    /// The server-assigned message identifier, when supplied.
    pub(crate) msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    ///
    /// This also accepts ZNC's `znc.in/server-time-iso` tag, which is not part of any `IRCv3`
    /// specification.
    pub server_time: Option<ServerTime>,
    /// Time at which the line was received, from this device's clock.
    pub received_at: SystemTime,
    /// All message tags in wire order.
    pub tags: Tags,
    pub(crate) receipt: crate::message_ref::Receipt,
}

macro_rules! ctcp_return_address {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl $ty {
                /// Returns the address that reaches the recipients of the CTCP line.
                ///
                /// See [`Conversation`] for status-prefixed line guidance.
                ///
                /// ```compile_fail
                /// # fn answer(client: &papo::Client, reply: &papo::event::CtcpReply) {
                /// let _ = client.reply(reply.return_address(), "automatic response");
                /// # }
                /// ```
                #[must_use]
                pub fn return_address(&self) -> crate::NoticeRef {
                    crate::message_ref::from_ctcp_reply(self)
                }
            }
        )+
    };
}

ctcp_return_address!(CtcpReply);

impl CtcpQuery {
    /// Returns the address that reaches the recipients of the CTCP line.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn return_address(&self) -> crate::MessageRef {
        crate::MessageRef::from(self)
    }
}

macro_rules! conversation_origin {
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

conversation_origin!(CtcpQuery, CtcpReply);

macro_rules! ctcp_reference_accessors {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl $ty {
                /// Returns the conversation captured from the received line.
                #[must_use]
                pub const fn conversation(&self) -> &Conversation {
                    &self.conversation
                }

                /// Returns the server-assigned message identifier, when supplied.
                #[must_use]
                pub const fn msgid(&self) -> Option<&MessageId> {
                    self.msgid.as_ref()
                }
            }
        )+
    };
}

ctcp_reference_accessors!(CtcpQuery, CtcpReply);

/// Who a person on a line is relative to you: an event's actor, or the user it is about.
///
/// papo decides it when the line arrives, from the nick and account you held at that moment,
/// and stores it in the event. Read it instead of comparing the sender with your own nick.
/// [`Events::next`](crate::Events::next) pairs each event with the state captured right after
/// it was applied, so a rename of yours that comes later does not change the comparison. It is
/// still wrong in these cases, where papo answers correctly:
///
/// - Your own `NICK`: the paired state already holds your new nick, so comparing it with
///   [`NickChanged::old`](crate::event::NickChanged::old) says someone else renamed. papo says
///   [`Party::Me`].
/// - Your account on another connection, such as another device behind a bouncer: it can use a
///   different nick, but the line's `account` tag matches yours, so it is [`Party::Me`].
/// - History: nobody can prove who held a nick back then, so a nick comparison guesses. papo
///   says [`Party::Unknown`] unless the replayed line's `account` tag proves who sent it.
///
/// A live line from a server has no nick to compare; papo says [`Party::Other`].
///
/// Comparing with [`Client::snapshot`](crate::Client::snapshot), the latest state instead of
/// the paired one, also goes wrong whenever your nick changed after the event.
///
/// ```
/// use papo::{Message, Party};
///
/// fn echo(message: &Message) -> Option<String> {
///     if message.actor_party == Party::Me {
///         return None;
///     }
///     Some(format!("you said: {}", message.text))
/// }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Party {
    /// The person is you.
    Me,
    /// The person is another user or a server.
    Other,
    /// The protocol did not prove who the person is relative to you.
    Unknown,
}

impl Party {
    pub(crate) const fn from_core(value: papo_core::Party) -> Self {
        match value {
            papo_core::Party::Me => Self::Me,
            papo_core::Party::Other => Self::Other,
            _ => Self::Unknown,
        }
    }
}

/// An incoming `PRIVMSG` or CTCP `ACTION` in a channel or private conversation.
///
/// This type cannot represent notices or server broadcasts. Inspect [`Self::actor_party`] to
/// tell your own messages from other users' messages.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Message {
    /// The user that sent the message.
    pub actor: crate::UserHandle,
    /// Who the actor is relative to you.
    pub actor_party: Party,
    /// The chat conversation to which the message belongs.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub(crate) conversation: Conversation,
    /// The lowest-ranked `STATUSMSG` prefix on the wire address, moved to
    /// [`Restricted::audience`] before delivery.
    pub(crate) status: Option<Prefix>,
    /// Display text with IRC formatting removed.
    pub text: String,
    /// Text with IRC formatting retained and the CTCP wrapper removed from an `ACTION`.
    pub raw_text: String,
    /// The semantic message kind.
    pub kind: MessageKind,
    /// The server-assigned message identifier, when supplied.
    pub(crate) msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    ///
    /// This also accepts ZNC's `znc.in/server-time-iso` tag, which is not part of any `IRCv3`
    /// specification.
    pub server_time: Option<ServerTime>,
    /// Time at which the line was received, from this device's clock.
    pub received_at: SystemTime,
    /// The identifier referenced by a reply tag.
    pub in_reply_to: Option<MessageId>,
    /// Channel context named by this private message; see [`Self::channel_context`].
    pub(crate) channel_context: Option<Conversation>,
    /// Nickname of the client that submitted a relayed message.
    #[cfg(feature = "unstable")]
    pub relayed_by: Option<papo_proto::Nick>,
    /// All message tags in wire order.
    pub tags: Tags,
    pub(crate) receipt: crate::message_ref::Receipt,
}

/// An incoming `NOTICE` from another user.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Notice {
    /// The user that sent the notice.
    pub actor: crate::UserHandle,
    /// Who the actor is relative to you.
    pub actor_party: Party,
    /// The chat conversation to which the notice belongs.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub conversation: Conversation,
    /// The lowest-ranked `STATUSMSG` prefix on the wire address, moved to
    /// [`Restricted::audience`] before delivery.
    pub(crate) status: Option<Prefix>,
    /// Display text with IRC formatting removed.
    pub text: String,
    /// Text with IRC formatting retained and the CTCP wrapper removed from an `ACTION`.
    pub raw_text: String,
    /// The semantic notice kind.
    pub kind: MessageKind,
    /// The server-assigned message identifier, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    ///
    /// This also accepts ZNC's `znc.in/server-time-iso` tag, which is not part of any `IRCv3`
    /// specification.
    pub server_time: Option<ServerTime>,
    /// Time at which the line was received, from this device's clock.
    pub received_at: SystemTime,
    /// The identifier referenced by a reply tag.
    pub in_reply_to: Option<MessageId>,
    /// Channel context named by this private notice.
    ///
    /// On a live notice, this proves that the actor and client shared the channel at receipt, not
    /// that the actor held any channel rank. On a history page item it is only the valid channel
    /// the line names, because past membership cannot be checked. Context that failed these
    /// checks remains available in [`Self::tags`]. It is the conversation to file the notice
    /// under, not an address: answer the notice through its [`Self::return_address`]; see
    /// [`Conversation`] for audience guidance.
    pub channel_context: Option<Conversation>,
    /// Nickname of the client that submitted a relayed notice.
    #[cfg(feature = "unstable")]
    pub relayed_by: Option<papo_proto::Nick>,
    /// All message tags in wire order.
    pub tags: Tags,
    pub(crate) receipt: crate::message_ref::Receipt,
}

/// A message or notice reconstructed from a `draft/multiline` batch that ended early.
/// Applications may show its text as an explicitly incomplete item. They must not treat it as a
/// mention, command, or reply target: later lines that could change its meaning never arrived.
/// It therefore has no return address or [`crate::MessageRef`], including for selecting an
/// explicit [`crate::MarkReadRequest::up_to`] boundary. Its server time still contributes to the
/// conversation boundary used by [`crate::Client::mark_read`] without `up_to`.
///
/// ```compile_fail
/// # fn reply(incomplete: &papo::Incomplete) {
/// let _ = incomplete.return_address();
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Incomplete {
    /// A partial user message.
    Message(Message),
    /// A partial user notice.
    Notice(Notice),
}

impl Incomplete {
    /// Returns the message when this is [`Self::Message`].
    #[must_use]
    pub const fn as_message(&self) -> Option<&Message> {
        if let Self::Message(message) = self {
            Some(message)
        } else {
            None
        }
    }

    /// Returns the notice when this is [`Self::Notice`].
    #[must_use]
    pub const fn as_notice(&self) -> Option<&Notice> {
        if let Self::Notice(notice) = self {
            Some(notice)
        } else {
            None
        }
    }

    /// Returns the conversation captured from the received lines.
    /// See [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn conversation(&self) -> &Conversation {
        match self {
            Self::Message(message) => message.conversation(),
            Self::Notice(notice) => &notice.conversation,
        }
    }
}

/// A notice or broadcast sent by a server or to a server-wide audience.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ServerNotice {
    /// The server or operator that sent the notice.
    pub actor: Source,
    /// Who the actor is relative to you.
    pub actor_party: Party,
    /// The command that carried the notice.
    pub kind: ServerNoticeKind,
    /// The notice's addressed audience.
    pub to: Audience,
    /// The lowest-ranked `STATUSMSG` prefix on a channel audience, moved to
    /// [`Restricted::audience`] before delivery.
    pub(crate) status: Option<Prefix>,
    /// Display text with IRC formatting removed.
    pub text: String,
    /// Text with IRC formatting retained.
    pub raw_text: String,
    /// The server-assigned message identifier, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    ///
    /// This also accepts ZNC's `znc.in/server-time-iso` tag, which is not part of any `IRCv3`
    /// specification.
    pub server_time: Option<ServerTime>,
    /// Time at which the line was received, from this device's clock.
    pub received_at: SystemTime,
    /// All message tags in wire order.
    pub tags: Tags,
    pub(crate) receipt: crate::message_ref::Receipt,
}

/// A parsed IRC line without a more specific typed event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Unparsed {
    /// The original parsed line.
    pub line: ProtoLine,
    /// The server-assigned message identifier, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    ///
    /// This also accepts ZNC's `znc.in/server-time-iso` tag, which is not part of any `IRCv3`
    /// specification.
    pub server_time: Option<ServerTime>,
    /// Time at which the line was received, from this device's clock.
    pub received_at: SystemTime,
}

impl std::ops::Deref for Unparsed {
    type Target = ProtoLine;

    fn deref(&self) -> &Self::Target {
        &self.line
    }
}

struct MessageMetadata {
    actor: Source,
    actor_party: Party,
    conversation: Conversation,
    status: Option<Prefix>,
    receipt: crate::message_ref::Receipt,
    msgid: Option<MessageId>,
    server_time: Option<ServerTime>,
    received_at: SystemTime,
    reply_to: Option<MessageId>,
    #[cfg(feature = "unstable")]
    relayed_by: Option<papo_proto::Nick>,
    tags: Tags,
}

#[allow(clippy::too_many_arguments)]
fn ctcp_from_line(
    raw: ProtoLine,
    batch_type: Option<&str>,
    actor_party: Party,
    conversation: Conversation,
    status: Option<Prefix>,
    isupport: &Isupport,
    account_tag_enabled: bool,
    prove_not_bot: bool,
    received_at: SystemTime,
    reply_token: Option<papo_core::ReplyToken>,
) -> Result<Event, ProtoLine> {
    let message = raw.as_ref();
    let Some((command, params, reply)) = ctcp_parts(&message) else {
        return Err(raw);
    };
    let Some(metadata) = message_metadata(
        &message,
        batch_type,
        actor_party,
        conversation,
        status,
        isupport,
        account_tag_enabled,
        prove_not_bot,
        received_at,
        reply_token,
    ) else {
        return Err(raw);
    };
    let Source::User(actor) = metadata.actor else {
        return Err(raw);
    };
    if reply {
        Ok(Event::CtcpReply(CtcpReply {
            actor,
            actor_party: metadata.actor_party,
            conversation: metadata.conversation,
            status: metadata.status,
            command,
            params,
            msgid: metadata.msgid,
            server_time: metadata.server_time,
            received_at: metadata.received_at,
            tags: metadata.tags,
            receipt: metadata.receipt,
        }))
    } else {
        Ok(Event::CtcpQuery(CtcpQuery {
            actor,
            actor_party: metadata.actor_party,
            conversation: metadata.conversation,
            status: metadata.status,
            command,
            params,
            msgid: metadata.msgid,
            server_time: metadata.server_time,
            received_at: metadata.received_at,
            tags: metadata.tags,
            receipt: metadata.receipt,
        }))
    }
}

fn ctcp_parts(message: &LineRef<'_>) -> Option<(crate::Command, Option<String>, bool)> {
    let reply = match () {
        () if message.command_is("PRIVMSG") => false,
        () if message.command_is("NOTICE") => true,
        () => return None,
    };
    let ctcp = ProtoCtcp::parse(message.param(1)?)?;
    (!ctcp.command.eq_ignore_ascii_case("ACTION")).then(|| {
        (
            ctcp.command.to_owned().into(),
            ctcp.params.map(str::to_owned),
            reply,
        )
    })
}

fn broadcast_from_line(
    raw: ProtoLine,
    actor_party: Party,
    origin: crate::Origin,
    isupport: &Isupport,
    account_tag_enabled: bool,
    prove_not_bot: bool,
    received_at: SystemTime,
) -> Result<Event, ProtoLine> {
    let message = raw.as_ref();
    let to = match message.param(0) {
        Some("*") => Broadcast::Everyone,
        Some(target) if target.starts_with('$') => {
            let mask = origin.__mask(isupport.casemapping(), target);
            if mask.check().is_err() {
                return Err(raw);
            }
            Broadcast::Mask(mask)
        }
        _ => return Err(raw),
    };
    let envelope = line_envelope_with_account_tag(
        &message,
        isupport.casemapping(),
        origin,
        account_tag_enabled,
        received_at,
    );
    let Some(Source::User(actor)) = source::from_line(
        source::LinePrefix::Raw(message.raw_source().ok_or_else(|| raw.clone())?),
        &envelope,
        isupport.casemapping(),
        origin,
        prove_not_bot,
    ) else {
        return Err(raw);
    };
    let tags = Tags::from_message(&message);
    if message.command_is("TAGMSG") {
        return Ok(Event::BroadcastTagMsg(BroadcastTagMsg {
            actor,
            actor_party,
            to,
            msgid: tag_owned(&message, &["msgid"]).map(MessageId::from),
            tags,
            received_at,
        }));
    }
    let Some((command, params, false)) = ctcp_parts(&message) else {
        return Err(raw);
    };
    Ok(Event::BroadcastCtcp(BroadcastCtcp {
        actor,
        actor_party,
        to,
        command,
        params,
        msgid: envelope.msgid,
        server_time: envelope.server_time,
        received_at: envelope.received_at,
        tags,
    }))
}

fn is_server_notice_line(message: &LineRef<'_>) -> bool {
    message.command_is("WALLOPS")
        || message
            .param(0)
            .is_some_and(|target| target == "*" || target.starts_with('$'))
        || matches!(message.source(), Some(papo_proto::Source::Server(_)))
}

pub(crate) fn incoming_from_core(
    received: papo_core::ReceivedLine,
    origin: crate::Origin,
    isupport: &Isupport,
    account_tag_enabled: bool,
    prove_not_bot: bool,
    received_at: SystemTime,
    handle_state: Option<&std::sync::Arc<std::sync::RwLock<std::sync::Arc<crate::State>>>>,
) -> Event {
    let reply_token = received.__reply_token();
    let tokens = LineTokens {
        actor: Some(received.__source_token()),
        peer: Some(reply_token),
    };
    let channel_guard = received.__channel_guard().cloned().filter(|guard| {
        handle_state.is_some_and(|state| {
            state
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .resolve_channel(guard)
                .is_some()
        })
    });
    if received.batch_type().is_some_and(|kind| {
        kind != crate::cap::LABELED_RESPONSE.as_str()
            && kind != crate::cap::MULTILINE.as_str()
            && !matches!(kind, "netsplit" | "netjoin")
    }) {
        return unparsed_from_line(received.into_line(), received_at);
    }
    let conversation = received.conversation().cloned();
    let status = received.status();
    let channel_context = received
        .channel_context()
        .cloned()
        .map(|channel| channel_context_handle(channel, origin, handle_state));
    let actor_party = Party::from_core(received.actor_party());
    let (raw, batch_type) = received.into_parts();
    let Some(conversation) = conversation else {
        let raw = match broadcast_from_line(
            raw,
            actor_party,
            origin,
            isupport,
            account_tag_enabled,
            prove_not_bot,
            received_at,
        ) {
            Ok(mut event) => {
                attach_message_handles(&mut event, tokens, handle_state, false);
                return event;
            }
            Err(raw) => raw,
        };
        if !is_server_notice_line(&raw.as_ref()) {
            return unparsed_from_line(raw, received_at);
        }
        let mut event = ServerNotice::from_line(
            raw,
            actor_party,
            None,
            None,
            origin,
            isupport,
            account_tag_enabled,
            prove_not_bot,
            received_at,
            Some(reply_token),
        )
        .map_or_else(
            |line| unparsed_from_line(line, received_at),
            Event::ServerNotice,
        );
        attach_message_handles(&mut event, tokens, handle_state, false);
        return event;
    };
    let conversation = Conversation::at_arrival(
        conversation,
        origin,
        handle_state,
        channel_guard.clone(),
        reply_token,
    );
    let mut event = message_event_from_line(
        raw,
        batch_type.as_deref(),
        actor_party,
        conversation,
        status,
        channel_context,
        isupport,
        account_tag_enabled,
        prove_not_bot,
        received_at,
        Some(reply_token),
    );
    attach_message_handles(&mut event, tokens, handle_state, false);
    event
}

fn channel_context_handle(
    channel: ChannelName,
    origin: crate::Origin,
    handle_state: Option<&std::sync::Arc<std::sync::RwLock<std::sync::Arc<crate::State>>>>,
) -> crate::ChannelHandle {
    let channel = origin.__channel(channel.casemapping(), channel.into_string());
    let guard = handle_state.map(|state| {
        state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .core
            .__channel_guard(&channel)
    });
    crate::ChannelHandle::from_parts(channel, guard, handle_state)
}

pub(crate) fn mark_incomplete(event: Event) -> Option<Event> {
    match event {
        Event::Message(message) => Some(Event::Incomplete(Incomplete::Message(message))),
        Event::Notice(notice) => Some(Event::Incomplete(Incomplete::Notice(notice))),
        _ => None,
    }
}

impl Message {
    /// Returns the conversation captured from the received line.
    #[must_use]
    pub const fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    /// Returns the server-assigned message identifier, when supplied.
    #[must_use]
    pub const fn msgid(&self) -> Option<&MessageId> {
        self.msgid.as_ref()
    }

    /// Returns the channel conversation this private message's context tag names.
    ///
    /// On a live message, the actor and client shared that channel at receipt; this says nothing
    /// about the actor's channel rank. On a history page item it is only the valid channel the
    /// line names, because past membership cannot be checked. Context that failed these checks
    /// remains available in [`Message::tags`].
    ///
    /// It is the conversation to file this private message under, not an address: sending to the
    /// channel would tell everyone in it what one person said privately. Answer the message with
    /// [`crate::Client::reply`], which stays private and keeps the context tag; see
    /// [`Conversation`] for audience guidance.
    ///
    /// ```compile_fail,E0277
    /// # async fn answer(client: &papo::Client, message: &papo::Message) -> Result<(), papo::Error> {
    /// if let Some(channel) = message.channel_context() {
    ///     client.privmsg(channel, "answering in public").await?;
    /// }
    /// # Ok(()) }
    /// ```
    #[must_use]
    pub const fn channel_context(&self) -> Option<&Conversation> {
        self.channel_context.as_ref()
    }

    /// Which `Client` produced this value (not the sender or the server).
    #[must_use]
    pub fn origin(&self) -> crate::Origin {
        crate::message_ref::conversation_origin(&self.conversation)
    }

    #[allow(dead_code)]
    pub(crate) fn new(
        raw: ProtoLine,
        isupport: &Isupport,
        nick: &str,
        received_at: SystemTime,
    ) -> Result<Self, ProtoLine> {
        let own_source = raw.as_ref().source().is_some_and(|source| {
            let papo_proto::Source::User { nick: source, .. } = source else {
                return false;
            };
            isupport.casemapping().nick(source) == nick
        });
        let actor_party = if own_source { Party::Me } else { Party::Other };
        let Ok((status, conversation)) =
            papo_core::classify_conversation(&raw.as_ref(), nick, isupport)
        else {
            return Err(raw);
        };
        let conversation = Conversation::from_core(conversation, crate::Origin::NONE, None, None);
        Self::new_with_context(
            raw,
            None,
            actor_party,
            conversation,
            status,
            None,
            isupport,
            false,
            false,
            received_at,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_context(
        raw: ProtoLine,
        batch_type: Option<&str>,
        actor_party: Party,
        conversation: Conversation,
        status: Option<Prefix>,
        channel_context: Option<crate::ChannelHandle>,
        isupport: &Isupport,
        account_tag_enabled: bool,
        prove_not_bot: bool,
        received_at: SystemTime,
        reply_token: Option<papo_core::ReplyToken>,
    ) -> Result<Self, ProtoLine> {
        let message = raw.as_ref();
        if !message.command_is("PRIVMSG") {
            return Err(raw);
        }
        let Some(raw_text) = message.param(1) else {
            return Err(raw);
        };
        let (kind, displayed) = match ProtoCtcp::parse(raw_text) {
            Some(ctcp) if ctcp.command.eq_ignore_ascii_case("ACTION") => {
                (MessageKind::Action, ctcp.params.unwrap_or(""))
            }
            Some(_) => return Err(raw),
            None => (MessageKind::Text, raw_text),
        };
        let text = strip_formatting(displayed).into_owned();
        let raw_text = displayed.to_owned();
        let Some(mut metadata) = message_metadata(
            &message,
            batch_type,
            actor_party,
            conversation,
            status,
            isupport,
            account_tag_enabled,
            prove_not_bot,
            received_at,
            reply_token,
        ) else {
            return Err(raw);
        };
        let Source::User(actor) = metadata.actor else {
            return Err(raw);
        };
        metadata
            .receipt
            .set_reference(metadata.msgid.iter().cloned(), channel_context.clone());
        let conversation = metadata.conversation;
        Ok(Self {
            actor,
            actor_party: metadata.actor_party,
            conversation,
            status: metadata.status,
            text,
            raw_text,
            kind,
            msgid: metadata.msgid,
            server_time: metadata.server_time,
            received_at: metadata.received_at,
            in_reply_to: metadata.reply_to,
            channel_context: channel_context.map(Conversation::Channel),
            #[cfg(feature = "unstable")]
            relayed_by: metadata.relayed_by,
            tags: metadata.tags,
            receipt: metadata.receipt,
        })
    }

    /// Returns the formatted spans parsed from [`Self::raw_text`].
    pub fn spans(&self) -> impl Iterator<Item = crate::Span<'_>> {
        papo_proto::Formatted::new(&self.raw_text)
    }

    /// Returns the address that reaches the recipients of the message.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn return_address(&self) -> crate::MessageRef {
        crate::MessageRef::from(self)
    }

    /// Reports whether this message's actor is this client's user.
    #[must_use]
    pub fn is_own(&self) -> bool {
        self.actor_party == Party::Me
    }
}

impl Notice {
    /// Which `Client` produced this value (not the sender or the server).
    #[must_use]
    pub fn origin(&self) -> crate::Origin {
        crate::message_ref::conversation_origin(&self.conversation)
    }

    /// Returns the address that reaches the recipients of the notice.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    ///
    /// A notice address is not a message reference and cannot be passed to [`Client::reply`]:
    ///
    /// ```compile_fail
    /// # fn answer(client: &papo::Client, notice: &papo::event::Notice) {
    /// let _ = client.reply(notice.return_address(), "automatic response");
    /// # }
    /// ```
    ///
    /// It also cannot be used as a reaction reference:
    ///
    /// ```compile_fail
    /// # fn react(client: &papo::Client, notice: &papo::event::Notice) {
    /// let _ = client.react(notice.return_address(), "✅");
    /// # }
    /// ```
    #[must_use]
    pub fn return_address(&self) -> crate::NoticeRef {
        crate::message_ref::from_notice(self)
    }

    #[allow(clippy::too_many_arguments)]
    fn from_line(
        raw: ProtoLine,
        batch_type: Option<&str>,
        actor_party: Party,
        conversation: Conversation,
        status: Option<Prefix>,
        channel_context: Option<crate::ChannelHandle>,
        isupport: &Isupport,
        account_tag_enabled: bool,
        prove_not_bot: bool,
        received_at: SystemTime,
        reply_token: Option<papo_core::ReplyToken>,
    ) -> Result<Self, ProtoLine> {
        let message = raw.as_ref();
        if !message.command_is("NOTICE") {
            return Err(raw);
        }
        let Some(raw_text) = message.param(1) else {
            return Err(raw);
        };
        let (kind, displayed) = match ProtoCtcp::parse(raw_text) {
            Some(ctcp) if ctcp.command.eq_ignore_ascii_case("ACTION") => {
                (MessageKind::Action, ctcp.params.unwrap_or(""))
            }
            Some(_) => return Err(raw),
            None => (MessageKind::Text, raw_text),
        };
        let text = strip_formatting(displayed).into_owned();
        let raw_text = displayed.to_owned();
        let Some(mut metadata) = message_metadata(
            &message,
            batch_type,
            actor_party,
            conversation,
            status,
            isupport,
            account_tag_enabled,
            prove_not_bot,
            received_at,
            reply_token,
        ) else {
            return Err(raw);
        };
        let Source::User(actor) = metadata.actor else {
            return Err(raw);
        };
        metadata
            .receipt
            .set_reference(metadata.msgid.iter().cloned(), channel_context.clone());
        let conversation = metadata.conversation;
        Ok(Self {
            actor,
            actor_party: metadata.actor_party,
            conversation,
            status: metadata.status,
            text,
            raw_text,
            kind,
            msgid: metadata.msgid,
            server_time: metadata.server_time,
            received_at: metadata.received_at,
            in_reply_to: metadata.reply_to,
            channel_context: channel_context.map(Conversation::Channel),
            #[cfg(feature = "unstable")]
            relayed_by: metadata.relayed_by,
            tags: metadata.tags,
            receipt: metadata.receipt,
        })
    }

    /// Returns the formatted spans parsed from [`Self::raw_text`].
    pub fn spans(&self) -> impl Iterator<Item = crate::Span<'_>> {
        papo_proto::Formatted::new(&self.raw_text)
    }
}

impl ServerNotice {
    /// Returns the address that reaches the recipients of the notice.
    ///
    /// Channel notices preserve their exact status-prefixed audience. Broadcasts, `WALLOPS`, and
    /// notices addressed only to this client have no return address.
    ///
    /// A returned notice address cannot be used as a message reference:
    ///
    /// ```compile_fail
    /// # fn answer(client: &papo::Client, notice: &papo::event::ServerNotice) {
    /// if let Some(address) = notice.return_address() {
    ///     let _ = client.reply(address, "automatic response");
    /// }
    /// # }
    /// ```
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn return_address(&self) -> Option<crate::NoticeRef> {
        matches!(self.to, Audience::Channel(_))
            .then(|| crate::message_ref::from_server_notice(self))
    }

    #[allow(clippy::too_many_arguments)]
    fn from_line(
        raw: ProtoLine,
        actor_party: Party,
        conversation: Option<&Conversation>,
        status: Option<Prefix>,
        origin: crate::Origin,
        isupport: &Isupport,
        account_tag_enabled: bool,
        prove_not_bot: bool,
        received_at: SystemTime,
        reply_token: Option<papo_core::ReplyToken>,
    ) -> Result<Self, ProtoLine> {
        let message = raw.as_ref();
        let kind = if message.command_is("WALLOPS") {
            ServerNoticeKind::Wallops
        } else if message.command_is("NOTICE") {
            ServerNoticeKind::Notice
        } else if message.command_is("PRIVMSG") {
            ServerNoticeKind::Privmsg
        } else {
            return Err(raw);
        };
        let wallops = kind == ServerNoticeKind::Wallops;
        let Some(raw_text) = message.param(usize::from(!wallops)) else {
            return Err(raw);
        };
        let to = if let Some(Conversation::Channel(channel)) = conversation {
            Audience::Channel(channel.clone())
        } else if wallops || message.param(0) == Some("*") {
            Audience::Broadcast(Broadcast::Everyone)
        } else if let Some(target) = message.param(0).filter(|target| target.starts_with('$')) {
            Audience::Broadcast(Broadcast::Mask(
                origin.__mask(isupport.casemapping(), target),
            ))
        } else if let Some(target) = message
            .param(0)
            .filter(|target| isupport.is_channel(target))
        {
            Audience::Channel(crate::ChannelHandle::from_parts(
                origin.__channel(isupport.casemapping(), target),
                None,
                None,
            ))
        } else {
            Audience::Me
        };
        let Some(prefix) = message.raw_source() else {
            return Err(raw);
        };
        let envelope = line_envelope_with_account_tag(
            &message,
            isupport.casemapping(),
            origin,
            account_tag_enabled,
            received_at,
        );
        let Some(actor) = source::from_line(
            source::LinePrefix::Raw(prefix),
            &envelope,
            isupport.casemapping(),
            origin,
            prove_not_bot,
        ) else {
            return Err(raw);
        };
        let receipt = match &to {
            Audience::Channel(channel) => crate::message_ref::Receipt::new(
                actor.clone(),
                &Conversation::Channel(channel.clone()),
                reply_token,
                message
                    .param(0)
                    .map_or_else(|| channel.name().to_string(), str::to_owned),
            ),
            Audience::Broadcast(_) | Audience::Me => {
                crate::message_ref::Receipt::without_recipient(
                    actor.clone(),
                    reply_token,
                    "*".to_owned(),
                    origin,
                )
            }
        };
        Ok(Self {
            actor,
            actor_party,
            kind,
            to,
            status,
            text: strip_formatting(raw_text).into_owned(),
            raw_text: raw_text.to_owned(),
            msgid: envelope.msgid,
            server_time: envelope.server_time,
            received_at: envelope.received_at,
            tags: Tags::from_message(&message),
            receipt,
        })
    }

    /// Returns the formatted spans parsed from [`Self::raw_text`].
    pub fn spans(&self) -> impl Iterator<Item = crate::Span<'_>> {
        papo_proto::Formatted::new(&self.raw_text)
    }
}

#[allow(clippy::too_many_arguments)]
fn message_event_from_line(
    raw: ProtoLine,
    batch_type: Option<&str>,
    actor_party: Party,
    conversation: Conversation,
    status: Option<Prefix>,
    channel_context: Option<crate::ChannelHandle>,
    isupport: &Isupport,
    account_tag_enabled: bool,
    prove_not_bot: bool,
    received_at: SystemTime,
    reply_token: Option<papo_core::ReplyToken>,
) -> Event {
    if is_server_notice_line(&raw.as_ref()) {
        let origin = crate::message_ref::conversation_origin(&conversation);
        return ServerNotice::from_line(
            raw,
            actor_party,
            Some(&conversation),
            status,
            origin,
            isupport,
            account_tag_enabled,
            prove_not_bot,
            received_at,
            reply_token,
        )
        .map_or_else(
            |line| unparsed_from_line(line, received_at),
            Event::ServerNotice,
        );
    }
    let raw = match ctcp_from_line(
        raw,
        batch_type,
        actor_party,
        conversation.clone(),
        status,
        isupport,
        account_tag_enabled,
        prove_not_bot,
        received_at,
        reply_token,
    ) {
        Ok(event) => return event,
        Err(raw) => raw,
    };
    if raw.as_ref().command_is("NOTICE") {
        return Notice::from_line(
            raw,
            batch_type,
            actor_party,
            conversation,
            status,
            channel_context,
            isupport,
            account_tag_enabled,
            prove_not_bot,
            received_at,
            reply_token,
        )
        .map_or_else(|line| unparsed_from_line(line, received_at), Event::Notice);
    }
    Message::new_with_context(
        raw,
        batch_type,
        actor_party,
        conversation,
        status,
        channel_context,
        isupport,
        account_tag_enabled,
        prove_not_bot,
        received_at,
        reply_token,
    )
    .map_or_else(|line| unparsed_from_line(line, received_at), Event::Message)
}

pub(crate) fn historical_message_event(
    raw: ProtoLine,
    actor_party: Party,
    history_conversation: Option<&Conversation>,
    client: &Client,
    received_at: SystemTime,
) -> Event {
    let snapshot = client.state();
    let Ok((status, conversation)) = papo_core::classify_conversation(
        &raw.as_ref(),
        snapshot.nick().as_str(),
        snapshot.isupport().inner(),
    ) else {
        return unparsed_from_line(raw, received_at);
    };
    let channel_context = papo_core::claimed_channel_context(
        &raw.as_ref(),
        &conversation,
        snapshot.isupport().inner(),
        client.origin(),
    )
    .map(|channel| channel_context_handle(channel, client.origin(), Some(&client.state)));
    let conversation =
        Conversation::from_core_with_state(conversation, client.origin(), &client.state, &snapshot);
    let conversation = historical_conversation(
        &raw.as_ref(),
        actor_party,
        conversation,
        history_conversation,
        snapshot.nick(),
        snapshot.casemapping(),
    );
    let reply_token = snapshot.reply_token(&conversation);
    message_event_from_line(
        raw,
        None,
        actor_party,
        conversation,
        status,
        channel_context,
        snapshot.isupport().inner(),
        snapshot.caps().contains_key(&crate::cap::ACCOUNT_TAG),
        false,
        received_at,
        reply_token,
    )
}

pub(crate) fn historical_conversation(
    message: &LineRef<'_>,
    actor_party: Party,
    conversation: Conversation,
    history_conversation: Option<&Conversation>,
    own_nick: &crate::Nick,
    mapping: Casemapping,
) -> Conversation {
    if !matches!(conversation, Conversation::Private(_)) {
        return crate::message_ref::stamp_conversation(conversation, own_nick.origin());
    }
    if let Some(Conversation::Private(user)) = history_conversation {
        return Conversation::Private(crate::UserHandle::from_parts(
            own_nick.origin().__nick(mapping, user.nick().as_str()),
            user.account().clone(),
            None,
            None,
        ));
    }
    if actor_party != Party::Me
        && message
            .param(0)
            .is_some_and(|target| own_nick.origin().__nick(mapping, target) != *own_nick)
        && let Some(papo_proto::Source::User { nick, .. }) = message.source()
    {
        return Conversation::Private(crate::UserHandle::from_nick(
            own_nick.origin().__nick(mapping, nick),
        ));
    }
    crate::message_ref::stamp_conversation(conversation, own_nick.origin())
}

#[allow(clippy::too_many_arguments)]
fn message_metadata(
    message: &LineRef<'_>,
    _batch_type: Option<&str>,
    actor_party: Party,
    conversation: Conversation,
    status: Option<Prefix>,
    isupport: &Isupport,
    account_tag_enabled: bool,
    prove_not_bot: bool,
    received_at: SystemTime,
    reply_token: Option<papo_core::ReplyToken>,
) -> Option<MessageMetadata> {
    let origin = match &conversation {
        Conversation::Channel(channel) => channel.origin(),
        Conversation::Private(user) => user.origin(),
    };
    let envelope = line_envelope_with_account_tag(
        message,
        isupport.casemapping(),
        origin,
        account_tag_enabled,
        received_at,
    );
    let actor = source::from_line(
        source::LinePrefix::Raw(message.raw_source()?),
        &envelope,
        isupport.casemapping(),
        origin,
        prove_not_bot,
    )?;
    let tags = Tags::from_message(message);
    let reply_to = tag_owned(message, &["+reply", "+draft/reply"]).map(MessageId::from);
    #[cfg(feature = "unstable")]
    let relayed_by = tag_owned(message, &[crate::cap::RELAYMSG.as_str()])
        .map(|nick| origin.__nick(isupport.casemapping(), nick));
    let return_address = return_address(message, &conversation);
    let mut receipt =
        crate::message_ref::Receipt::new(actor.clone(), &conversation, reply_token, return_address);
    receipt.set_reference(envelope.msgid.iter().cloned(), None);
    Some(MessageMetadata {
        actor,
        actor_party,
        conversation,
        status,
        receipt,
        msgid: envelope.msgid,
        server_time: envelope.server_time,
        received_at: envelope.received_at,
        reply_to,
        #[cfg(feature = "unstable")]
        relayed_by,
        tags,
    })
}

pub(crate) fn return_address(message: &LineRef<'_>, conversation: &Conversation) -> String {
    match conversation {
        Conversation::Channel(channel) => message
            .param(0)
            .map_or_else(|| channel.name().to_string(), str::to_owned),
        Conversation::Private(user) => user.nick().to_string(),
    }
}

fn account_from_tag(
    message: &LineRef<'_>,
    casemapping: Casemapping,
    origin: crate::Origin,
    account_tag_enabled: bool,
) -> Account {
    tag_owned(message, &["account"]).map_or_else(
        || {
            if account_tag_enabled {
                Account::LoggedOut
            } else {
                Account::Unknown
            }
        },
        |account| Account::LoggedIn(origin.__account(casemapping, account)),
    )
}

pub(crate) struct EventEnvelope {
    pub(crate) msgid: Option<MessageId>,
    pub(crate) server_time: Option<ServerTime>,
    pub(crate) received_at: SystemTime,
    pub(crate) account: Account,
}

impl EventEnvelope {
    fn with_account(mut self, account: Account) -> Self {
        if self.account == Account::Unknown {
            self.account = account;
        }
        self
    }
}

pub(crate) fn line_envelope_with_account_tag(
    line: &LineRef<'_>,
    casemapping: Casemapping,
    origin: crate::Origin,
    account_tag_enabled: bool,
    received_at: SystemTime,
) -> EventEnvelope {
    // NONSTANDARD(znc): ZNC uses its legacy tag instead of IRCv3 `time`, so playback keeps its
    // original timestamps.
    let server_time = tag_owned(line, &["time", "znc.in/server-time-iso"])
        .and_then(|value| ServerTime::from_stored(&value).ok());
    EventEnvelope {
        msgid: tag_owned(line, &["msgid"]).map(MessageId::from),
        server_time,
        received_at,
        account: account_from_tag(line, casemapping, origin, account_tag_enabled),
    }
}

pub(crate) fn joined_from_line(
    line: &LineRef<'_>,
    actor_party: Party,
    casemapping: Casemapping,
    origin: crate::Origin,
    account_tag_enabled: bool,
    prove_not_bot: bool,
    received_at: SystemTime,
) -> Option<Joined> {
    let join = papo_proto::Join::parse(line)?;
    let account = match (join.realname, join.account) {
        (None, _) => Account::Unknown,
        (Some(_), None) => Account::LoggedOut,
        (Some(_), Some(account)) => Account::LoggedIn(origin.__account(casemapping, account)),
    };
    let envelope =
        line_envelope_with_account_tag(line, casemapping, origin, account_tag_enabled, received_at)
            .with_account(account);
    let actor = source::from_line(
        source::LinePrefix::Raw(line.raw_source()?),
        &envelope,
        casemapping,
        origin,
        prove_not_bot,
    )?;
    let Source::User(actor) = actor else {
        return None;
    };
    Some(Joined {
        msgid: envelope.msgid,
        server_time: envelope.server_time,
        received_at: envelope.received_at,
        channel: crate::ChannelHandle::from_parts(
            origin.__channel(casemapping, join.channel),
            None,
            None,
        ),
        actor,
        actor_party,
        netjoin: None,
    })
}

fn core_envelope(
    envelope: &papo_core::LineEnvelope,
    casemapping: Casemapping,
    origin: crate::Origin,
    account_tag_enabled: bool,
    received_at: SystemTime,
) -> EventEnvelope {
    EventEnvelope {
        msgid: envelope.msgid().map(MessageId::from),
        server_time: envelope.server_time(),
        received_at,
        account: envelope.account().map_or_else(
            || {
                if envelope.__account_known() || account_tag_enabled {
                    Account::LoggedOut
                } else {
                    Account::Unknown
                }
            },
            |account| Account::LoggedIn(origin.__account(casemapping, account)),
        ),
    }
}

pub(crate) fn allowed_alias<'a>(
    denied: &papo_proto::ClientTagDeny<'_>,
    preferred: &'a str,
    fallback: &'a str,
) -> Option<&'a str> {
    (!denied.denies(preferred))
        .then_some(preferred)
        .or_else(|| (!denied.denies(fallback)).then_some(fallback))
}

fn tag_owned(message: &LineRef<'_>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| message.tag(key).map(std::borrow::Cow::into_owned))
}

pub(crate) fn unparsed_from_line(line: ProtoLine, received_at: SystemTime) -> Event {
    let message = line.as_ref();
    let msgid = tag_owned(&message, &["msgid"]).map(MessageId::from);
    // NONSTANDARD(znc): ZNC uses its legacy tag instead of IRCv3 `time`, so unparsed playback
    // lines keep their original timestamps.
    let server_time = tag_owned(&message, &["time", "znc.in/server-time-iso"])
        .and_then(|value| ServerTime::from_stored(&value).ok());
    Event::Other(Unparsed {
        line,
        msgid,
        server_time,
        received_at,
    })
}

/// An event emitted by an IRC client.
///
/// ```
/// fn summary(event: &papo::Event) -> &'static str {
///     match event {
///         papo::Event::Connected(_) => "connected",
///         papo::Event::Message(_) => "chat message",
///         papo::Event::Disconnected(_) => "disconnected",
///         _ => "another IRC event",
///     }
/// }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// The connection moved to another setup phase.
    Connecting(Connecting),
    /// The IRC connection is ready for use.
    Connected(Connected),
    /// Events were discarded because the event buffer was full.
    ///
    /// State-backed changes are discarded first and remain visible through [`Client::snapshot`].
    /// Message-like events are discarded last; their conversation gaps can be recovered with
    /// [`Client::history`] when the server supports history.
    Lagged(Lagged),
    /// A complete message of the day received while connecting.
    Motd(Motd),
    /// An incoming `PRIVMSG` or `ACTION` from any user.
    Message(Message),
    /// An incoming `NOTICE` from a user.
    Notice(Notice),
    /// A message or notice whose `draft/multiline` batch ended before its closing line.
    ///
    /// The text is safe to display as incomplete, but must not be interpreted as a mention,
    /// command, or reply target. Ignoring this variant preserves the former drop behavior.
    Incomplete(Incomplete),
    /// A server notice, broadcast, `*` message, or `WALLOPS`.
    ServerNotice(ServerNotice),
    /// A line sent to a status-prefixed channel address, such as `@#ops`.
    ///
    /// Such a line never arrives in another arm. See [`Conversation`] for status-prefixed line
    /// guidance.
    Restricted(Restricted),
    /// An incoming Client-to-Client Protocol query.
    CtcpQuery(CtcpQuery),
    /// An incoming Client-to-Client Protocol reply.
    CtcpReply(CtcpReply),
    /// A Client-to-Client Protocol query sent to a server-wide audience.
    BroadcastCtcp(BroadcastCtcp),
    /// A tag-only message sent to a server-wide audience.
    BroadcastTagMsg(BroadcastTagMsg),
    /// A line papo did not turn into a typed event.
    ///
    /// This includes unknown numerics, vendor commands, and new drafts. Match it only for display
    /// and logging; a future version may type it.
    Other(Unparsed),
    /// A tag-only message with no more specific typed event.
    TagMsg(TagMsg),
    /// A tag-only reaction or unreaction.
    Reacted(Reacted),
    /// A user's typing state changed.
    TypingChanged(TypingChanged),
    /// A message was redacted from a conversation's history.
    Redacted(Redacted),
    /// An unsolicited `IRCv3` standard reply.
    StandardReply(StandardReply),
    /// Account creation completed or requires verification.
    AccountCreation(crate::AccountOutcome),
    /// The server reported the account's persistence state.
    #[cfg(feature = "unstable")]
    PersistenceChanged(crate::PersistenceStatus),
    /// An upstream bouncer network was added, updated, or removed.
    #[cfg(feature = "unstable")]
    BouncerNetworkChanged(BouncerNetworkChanged),
    /// A user or channel's cached metadata changed.
    MetadataChanged(MetadataChanged),
    /// A completed server-initiated playback or automatic reconnect catch-up batch.
    ///
    /// Pages returned by [`crate::Client::history`] resolve their request futures and are not
    /// emitted as events. Insert page items by server time because live traffic can arrive while
    /// a batch remains open. Page items keep server delivery order, which is normally oldest to
    /// newest but is not sorted by papo.
    History(crate::HistoryPage),
    /// A conversation's read marker advanced or was first reported.
    ReadMarkerChanged(ReadMarkerChanged),
    /// A recoverable protocol or configuration problem occurred.
    Warning(Warning),
    /// The capabilities enabled on an established connection changed.
    CapsChanged(CapsChanged),
    /// Optional behaviors available on an established connection changed.
    FeaturesChanged(FeaturesChanged),
    /// Server-defined name comparison rules changed between connections.
    CasemappingChanged(CasemappingChanged),
    /// A channel join configured or remembered by papo was refused.
    AutoJoinFailed(AutoJoinFailed),
    /// A user joined a channel shared with the client.
    Joined(Joined),
    /// Initial channel membership synchronization completed.
    ChannelSynced(ChannelSynced),
    /// A channel's member facts were refreshed by a `WHO` sweep.
    MembersRefreshed(MembersRefreshed),
    /// A user left a shared channel with `PART`.
    Parted(Parted),
    /// A user was removed from a shared channel with `KICK`.
    Kicked(Kicked),
    /// A user was invited to a channel.
    Invited(Invited),
    /// A user sharing channels with the client disconnected.
    Quit(Quit),
    /// A user changed nickname.
    NickChanged(NickChanged),
    /// A joined channel was renamed.
    ChannelRenamed(ChannelRenamed),
    /// A user's account state changed.
    AccountChanged(AccountChanged),
    /// A user's away state changed.
    AwayChanged(AwayChanged),
    /// A user's username or hostname changed.
    HostChanged(HostChanged),
    /// A user's real name changed.
    RealnameChanged(RealnameChanged),
    /// A user or channel mode changed.
    ModeChanged(ModeChanged),
    /// A channel topic changed.
    TopicChanged(TopicChanged),
    /// A monitored nickname came online or changed its userhost.
    MonitorOnline(MonitorOnline),
    /// A monitored nickname went offline.
    MonitorOffline(MonitorOffline),
    /// The connection ended.
    Disconnected(Disconnected),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum OverflowPriority {
    State,
    Other,
    Message,
}

pub(crate) struct OverflowClass<'a> {
    pub(crate) priority: OverflowPriority,
    pub(crate) conversation: Option<&'a Conversation>,
}

variant_accessors! { Event;
    (Message, as_message, Message),
    (Notice, as_notice, Notice),
    (Incomplete, as_incomplete, Incomplete),
    (ServerNotice, as_server_notice, ServerNotice),
    (Restricted, as_restricted, Restricted),
    (CtcpQuery, as_ctcp_query, CtcpQuery),
    (CtcpReply, as_ctcp_reply, CtcpReply),
    (BroadcastCtcp, as_broadcast_ctcp, BroadcastCtcp),
    (BroadcastTagMsg, as_broadcast_tag_msg, BroadcastTagMsg),
    (History, as_history, crate::HistoryPage),
    (Connected, as_connected, Connected),
    (Lagged, as_lagged, Lagged),
    (Connecting, as_connecting, Connecting),
    (Motd, as_motd, Motd),
    (Disconnected, as_disconnected, Disconnected),
    (CapsChanged, as_caps_changed, CapsChanged),
    (FeaturesChanged, as_features_changed, FeaturesChanged),
    (CasemappingChanged, as_casemapping_changed, CasemappingChanged),
    (AutoJoinFailed, as_auto_join_failed, AutoJoinFailed),
    (Reacted, as_reacted, Reacted),
    (Redacted, as_redacted, Redacted),
    (TypingChanged, as_typing_changed, TypingChanged),
    (TagMsg, as_tag_msg, TagMsg),
    (ReadMarkerChanged, as_read_marker_changed, ReadMarkerChanged),
    (StandardReply, as_standard_reply, StandardReply),
    (Joined, as_joined, Joined),
    (ChannelSynced, as_channel_synced, ChannelSynced),
    (MembersRefreshed, as_members_refreshed, MembersRefreshed),
    (Parted, as_parted, Parted),
    (Kicked, as_kicked, Kicked),
    (Invited, as_invited, Invited),
    (TopicChanged, as_topic_changed, TopicChanged),
    (ModeChanged, as_mode_changed, ModeChanged),
    (ChannelRenamed, as_channel_renamed, ChannelRenamed),
    (Quit, as_quit, Quit),
    (NickChanged, as_nick_changed, NickChanged),
    (AwayChanged, as_away_changed, AwayChanged),
    (AccountChanged, as_account_changed, AccountChanged),
    (HostChanged, as_host_changed, HostChanged),
    (RealnameChanged, as_realname_changed, RealnameChanged),
    (MonitorOnline, as_monitor_online, MonitorOnline),
    (MonitorOffline, as_monitor_offline, MonitorOffline),
    (MetadataChanged, as_metadata_changed, MetadataChanged),
    (AccountCreation, as_account_creation, crate::AccountOutcome),
    (Warning, as_warning, Warning),
    (Other, as_other, Unparsed),
}

impl Event {
    /// Returns the conversation buffer to which this event belongs.
    ///
    /// Channel-addressed server notices belong to that channel. A channel rename belongs to its
    /// new name. Events spanning several channels, such as quits and nickname changes, return
    /// `None`; use their `channels` fields to file them in each affected buffer. See
    /// [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn conversation(&self) -> Option<Conversation> {
        match self {
            Self::Message(_)
            | Self::Notice(_)
            | Self::Incomplete(_)
            | Self::ServerNotice(_)
            | Self::Restricted(_)
            | Self::CtcpQuery(_)
            | Self::CtcpReply(_)
            | Self::TagMsg(_)
            | Self::Reacted(_)
            | Self::Redacted(_)
            | Self::TypingChanged(_)
            | Self::Other(_) => self.line_view().and_then(LineView::conversation),
            Self::MetadataChanged(event) => match &event.subject {
                crate::Subject::Channel(channel) => Some(Conversation::Channel(channel.clone())),
                crate::Subject::User(_) => None,
            },
            Self::History(event) => Some(event.conversation.clone()),
            Self::ReadMarkerChanged(event) => Some(event.conversation.clone()),
            Self::AutoJoinFailed(event) => Some(Conversation::Channel(event.channel.clone())),
            Self::Joined(event) => Some(Conversation::Channel(event.channel.clone())),
            Self::ChannelSynced(event) => Some(Conversation::Channel(event.channel.clone())),
            Self::MembersRefreshed(event) => Some(Conversation::Channel(event.channel.clone())),
            Self::Parted(event) => Some(Conversation::Channel(event.channel.clone())),
            Self::Kicked(event) => Some(Conversation::Channel(event.channel.clone())),
            Self::Invited(event) => Some(Conversation::Channel(event.channel.clone())),
            Self::ChannelRenamed(event) => Some(Conversation::Channel(event.channel.clone())),
            Self::ModeChanged(event) => match &event.subject {
                crate::Subject::Channel(channel) => Some(Conversation::Channel(channel.clone())),
                crate::Subject::User(_) => None,
            },
            Self::TopicChanged(event) => Some(Conversation::Channel(event.channel.clone())),
            Self::Connecting(_)
            | Self::Connected(_)
            | Self::Lagged(_)
            | Self::Motd(_)
            | Self::BroadcastCtcp(_)
            | Self::BroadcastTagMsg(_)
            | Self::StandardReply(_)
            | Self::AccountCreation(_)
            | Self::Warning(_)
            | Self::CapsChanged(_)
            | Self::FeaturesChanged(_)
            | Self::CasemappingChanged(_)
            | Self::Quit(_)
            | Self::NickChanged(_)
            | Self::AccountChanged(_)
            | Self::AwayChanged(_)
            | Self::HostChanged(_)
            | Self::RealnameChanged(_)
            | Self::MonitorOnline(_)
            | Self::MonitorOffline(_)
            | Self::Disconnected(_) => None,
            #[cfg(feature = "unstable")]
            Self::PersistenceChanged(_) | Self::BouncerNetworkChanged(_) => None,
        }
    }

    pub(crate) fn overflow_class(&self) -> OverflowClass<'_> {
        let (priority, conversation) = match self {
            Self::Message(event) => (OverflowPriority::Message, Some(&event.conversation)),
            Self::Notice(event) => (OverflowPriority::Message, Some(&event.conversation)),
            Self::Incomplete(event) => (OverflowPriority::Message, Some(event.conversation())),
            Self::CtcpQuery(event) => (OverflowPriority::Message, Some(&event.conversation)),
            Self::CtcpReply(event) => (OverflowPriority::Message, Some(&event.conversation)),
            Self::TagMsg(event) => (OverflowPriority::Message, Some(&event.conversation)),
            Self::Reacted(event) => (OverflowPriority::Message, Some(&event.conversation)),
            Self::Redacted(event) => (OverflowPriority::Message, Some(&event.conversation)),
            Self::BroadcastCtcp(_) | Self::BroadcastTagMsg(_) => (OverflowPriority::Message, None),
            Self::CapsChanged(_)
            | Self::FeaturesChanged(_)
            | Self::CasemappingChanged(_)
            | Self::MetadataChanged(_)
            | Self::ReadMarkerChanged(_)
            | Self::Joined(_)
            | Self::ChannelSynced(_)
            | Self::MembersRefreshed(_)
            | Self::Parted(_)
            | Self::Kicked(_)
            | Self::Quit(_)
            | Self::NickChanged(_)
            | Self::ChannelRenamed(_)
            | Self::AccountChanged(_)
            | Self::AwayChanged(_)
            | Self::HostChanged(_)
            | Self::RealnameChanged(_)
            | Self::ModeChanged(_)
            | Self::TopicChanged(_)
            | Self::MonitorOnline(_)
            | Self::MonitorOffline(_)
            | Self::TypingChanged(_) => (OverflowPriority::State, None),
            _ => (OverflowPriority::Other, None),
        };
        OverflowClass {
            priority,
            conversation,
        }
    }

    /// Returns the server-assigned identifier carried by this event, when supplied.
    #[must_use]
    pub fn msgid(&self) -> Option<&MessageId> {
        if let Some(line) = self.line_view() {
            return line.msgid();
        }
        match self {
            Self::BroadcastCtcp(event) => event.msgid.as_ref(),
            Self::BroadcastTagMsg(event) => event.msgid.as_ref(),
            Self::Joined(event) => event.msgid.as_ref(),
            Self::Parted(event) => event.msgid.as_ref(),
            Self::Kicked(event) => event.msgid.as_ref(),
            Self::Invited(event) => event.msgid.as_ref(),
            Self::Quit(event) => event.msgid.as_ref(),
            Self::NickChanged(event) => event.msgid.as_ref(),
            Self::ChannelRenamed(event) => event.msgid.as_ref(),
            Self::ModeChanged(event) => event.msgid.as_ref(),
            Self::TopicChanged(event) => event.msgid.as_ref(),
            _ => None,
        }
    }

    /// Returns the timestamp supplied by the server, when present and valid.
    ///
    /// This also accepts ZNC's `znc.in/server-time-iso` tag, which is not part of any `IRCv3`
    /// specification.
    #[must_use]
    pub fn server_time(&self) -> Option<ServerTime> {
        if let Some(line) = self.line_view() {
            return line.server_time();
        }
        match self {
            Self::BroadcastCtcp(event) => event.server_time,
            Self::BroadcastTagMsg(event) => tagged_from_tags(&event.tags),
            Self::Joined(event) => event.server_time,
            Self::Parted(event) => event.server_time,
            Self::Kicked(event) => event.server_time,
            Self::Quit(event) => event.server_time,
            Self::NickChanged(event) => event.server_time,
            Self::ModeChanged(event) => event.server_time,
            Self::TopicChanged(event) => event.server_time,
            Self::Invited(event) => event.server_time,
            Self::ChannelRenamed(event) => event.server_time,
            Self::AccountChanged(event) => event.server_time,
            Self::AwayChanged(event) => event.server_time,
            Self::HostChanged(event) => event.server_time,
            Self::RealnameChanged(event) => event.server_time,
            _ => None,
        }
    }

    /// Returns when this device's clock observed the event line, when timestamped.
    #[must_use]
    pub fn received_at(&self) -> Option<SystemTime> {
        if let Some(line) = self.line_view() {
            return Some(line.received_at());
        }
        match self {
            Self::BroadcastCtcp(event) => Some(event.received_at),
            Self::BroadcastTagMsg(event) => Some(event.received_at),
            Self::Joined(event) => Some(event.received_at),
            Self::Parted(event) => Some(event.received_at),
            Self::Kicked(event) => Some(event.received_at),
            Self::Invited(event) => Some(event.received_at),
            Self::Quit(event) => Some(event.received_at),
            Self::NickChanged(event) => Some(event.received_at),
            Self::ChannelRenamed(event) => Some(event.received_at),
            Self::AccountChanged(event) => Some(event.received_at),
            Self::AwayChanged(event) => Some(event.received_at),
            Self::HostChanged(event) => Some(event.received_at),
            Self::RealnameChanged(event) => Some(event.received_at),
            Self::ModeChanged(event) => Some(event.received_at),
            Self::TopicChanged(event) => Some(event.received_at),
            _ => None,
        }
    }
}

fn event_handle(
    handle: &crate::UserHandle,
    token: Option<papo_core::ReplyToken>,
    state: Option<&std::sync::Arc<std::sync::RwLock<std::sync::Arc<crate::State>>>>,
    historical: bool,
) -> crate::UserHandle {
    crate::UserHandle::from_event(
        handle.nick(),
        handle.account().clone(),
        token,
        state,
        historical,
    )
}

/// The identities papo-core recorded with a line for its actor and its private peer.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LineTokens {
    pub(crate) actor: Option<papo_core::ReplyToken>,
    pub(crate) peer: Option<papo_core::ReplyToken>,
}

fn attach_user_message_handle(
    actor: &mut crate::UserHandle,
    actor_party: Party,
    conversation: &mut Conversation,
    receipt: &mut crate::message_ref::Receipt,
    tokens: LineTokens,
    state: Option<&std::sync::Arc<std::sync::RwLock<std::sync::Arc<crate::State>>>>,
    historical: bool,
) {
    *actor = event_handle(actor, tokens.actor, state, historical);
    if let Conversation::Private(peer) = conversation {
        *peer = if actor_party == Party::Me {
            event_handle(peer, tokens.peer, state, historical)
        } else {
            actor.clone()
        };
    }
    receipt.update_event_actor(Source::User(actor.clone()), conversation);
}

pub(crate) fn attach_message_handles(
    event: &mut Event,
    tokens: LineTokens,
    state: Option<&std::sync::Arc<std::sync::RwLock<std::sync::Arc<crate::State>>>>,
    historical: bool,
) {
    macro_rules! user_event {
        ($event:expr) => {{
            let event = $event;
            attach_user_message_handle(
                &mut event.actor,
                event.actor_party,
                &mut event.conversation,
                &mut event.receipt,
                tokens,
                state,
                historical,
            );
        }};
    }
    match event {
        Event::Message(event) | Event::Incomplete(Incomplete::Message(event)) => {
            user_event!(event);
        }
        Event::Notice(event) | Event::Incomplete(Incomplete::Notice(event)) => {
            user_event!(event);
        }
        Event::CtcpQuery(event) => user_event!(event),
        Event::CtcpReply(event) => user_event!(event),
        Event::TagMsg(event) => user_event!(event),
        Event::Reacted(event) => user_event!(event),
        Event::TypingChanged(event) => user_event!(event),
        Event::Redacted(event) => {
            if let Source::User(actor) = &mut event.actor {
                *actor = event_handle(actor, tokens.actor, state, historical);
                if let Conversation::Private(peer) = &mut event.conversation {
                    *peer = if event.actor_party == Party::Me {
                        event_handle(peer, tokens.peer, state, historical)
                    } else {
                        actor.clone()
                    };
                }
                event
                    .receipt
                    .update_event_actor(Source::User(actor.clone()), &event.conversation);
            }
        }
        Event::ServerNotice(event) => {
            if let Source::User(actor) = &mut event.actor {
                *actor = event_handle(actor, tokens.actor, state, historical);
                let conversation = match &event.to {
                    Audience::Channel(channel) => Conversation::Channel(channel.clone()),
                    Audience::Me | Audience::Broadcast(_) => return,
                };
                event
                    .receipt
                    .update_event_actor(Source::User(actor.clone()), &conversation);
            }
        }
        Event::BroadcastCtcp(event) => {
            event.actor = event_handle(&event.actor, tokens.actor, state, historical);
        }
        Event::BroadcastTagMsg(event) => {
            event.actor = event_handle(&event.actor, tokens.actor, state, historical);
        }
        _ => {}
    }
}

fn tagged_from_tags(tags: &Tags) -> Option<ServerTime> {
    // NONSTANDARD(znc): ZNC uses its legacy tag instead of IRCv3 `time`, so tag-only events keep
    // their original timestamps.
    tags.get("time")
        .or_else(|| tags.get("znc.in/server-time-iso"))
        .and_then(|value| ServerTime::from_stored(value).ok())
}

#[cfg(feature = "unstable")]
impl Event {
    /// Returns the payload when this is [`Event::PersistenceChanged`].
    #[must_use]
    pub const fn as_persistence_changed(&self) -> Option<&crate::PersistenceStatus> {
        if let Self::PersistenceChanged(payload) = self {
            Some(payload)
        } else {
            None
        }
    }

    /// Returns the payload when this is [`Event::BouncerNetworkChanged`].
    #[must_use]
    pub const fn as_bouncer_network_changed(&self) -> Option<&BouncerNetworkChanged> {
        if let Self::BouncerNetworkChanged(payload) = self {
            Some(payload)
        } else {
            None
        }
    }
}

// This transient mapping keeps forwarding allocation-free for every event kind.
#[allow(clippy::large_enum_variant)]
pub(crate) enum CoreEvent {
    Forward(Event),
    Connected(Welcome),
    Line(CoreLine),
    Response(Response),
    RequestUntracked(RequestId),
    RequestTimedOut(RequestId),
    RequestFailed {
        id: RequestId,
        error: papo_core::RequestError,
    },
    RequestAborted {
        id: RequestId,
        transmitted: bool,
    },
    TransmitAborted(papo_core::TransmitToken),
    RequestRecipientGone(RequestId),
    TransmitRecipientGone(papo_core::TransmitToken),
    RequestCredentialsRefused(RequestId),
    TransmitCredentialsRefused(papo_core::TransmitToken),
    StsUpgrade(u16),
    StsPolicy(StsAction),
    History {
        conversation: papo_core::Conversation,
        items: Vec<papo_core::HistoryItem>,
        end: bool,
    },
    Close {
        kind: papo_core::CloseKind,
        reason: String,
    },
    RequestUnsupported {
        id: RequestId,
        unsupported: papo_core::Unsupported,
    },
    Unknown,
}

pub(crate) enum CoreLine {
    Complete(papo_core::ReceivedLine),
    Incomplete(papo_core::ReceivedLine),
}

impl CoreLine {
    pub(crate) fn into_parts(self) -> (papo_core::ReceivedLine, bool) {
        match self {
            Self::Complete(line) => (line, false),
            Self::Incomplete(line) => (line, true),
        }
    }

    pub(crate) fn into_incomplete(self) -> Option<papo_core::ReceivedLine> {
        match self {
            Self::Complete(_) => None,
            Self::Incomplete(line) => Some(line),
        }
    }
}

// Keeping the exhaustive forwarding table together makes omissions visible when core grows.
#[allow(dead_code, clippy::too_many_lines)]
pub(crate) fn from_core(
    event: papo_core::Event,
    casemapping: Casemapping,
    origin: crate::Origin,
    account_tag_enabled: bool,
    prove_not_bot: bool,
    received_at: SystemTime,
) -> CoreEvent {
    from_core_with_receipt(
        event,
        casemapping,
        origin,
        account_tag_enabled,
        prove_not_bot,
        received_at,
        None,
        None,
    )
}

fn private_reply_token(
    conversation: &papo_core::Conversation,
    peer_token: papo_core::ReplyToken,
) -> Option<papo_core::ReplyToken> {
    matches!(conversation, papo_core::Conversation::Private(_)).then_some(peer_token)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) fn from_core_with_receipt(
    event: papo_core::Event,
    casemapping: Casemapping,
    origin: crate::Origin,
    account_tag_enabled: bool,
    prove_not_bot: bool,
    received_at: SystemTime,
    reply_state: Option<&papo_core::State>,
    handle_state: Option<&std::sync::Arc<std::sync::RwLock<std::sync::Arc<crate::State>>>>,
) -> CoreEvent {
    let map_envelope = |envelope| {
        core_envelope(
            envelope,
            casemapping,
            origin,
            account_tag_enabled,
            received_at,
        )
    };
    let actor_from_raw = |prefix: &str, envelope: &EventEnvelope| {
        source::from_line(
            source::LinePrefix::Raw(prefix),
            envelope,
            casemapping,
            origin,
            prove_not_bot,
        )
    };
    let user_at_arrival =
        |nick, account, token| crate::UserHandle::at_arrival(nick, account, token, handle_state);
    let source_at_arrival = |source, token| match source {
        Source::User(user) => {
            Source::User(user_at_arrival(user.nick(), user.account().clone(), token))
        }
        Source::Server(server) => Source::Server(server),
    };
    let channel_handle = |channel: ChannelName| {
        let channel = origin.__channel(channel.casemapping(), channel.into_string());
        let guard = reply_state.and_then(|state| {
            state
                .channel(&channel)
                .map(|_| state.__channel_guard(&channel))
        });
        crate::ChannelHandle::from_parts(channel, guard, handle_state)
    };
    let conversation_handle = |conversation, peer_token| match conversation {
        papo_core::Conversation::Channel(channel) => Conversation::Channel(channel_handle(channel)),
        papo_core::Conversation::Private(nick) => Conversation::Private(user_at_arrival(
            origin.__nick(nick.casemapping(), nick.into_string()),
            Account::Unknown,
            peer_token,
        )),
        _ => unreachable!("core conversations have channel and private variants"),
    };
    let handle_from_raw =
        |prefix: &str, envelope: &EventEnvelope| match actor_from_raw(prefix, envelope) {
            Some(Source::User(user)) => Some(user),
            Some(Source::Server(_)) | None => None,
        };
    let mut actor_token = None;
    let mut peer_token = None;
    let mut mapped = match event {
        papo_core::Event::Connected(welcome) => return CoreEvent::Connected(welcome),
        papo_core::Event::Motd { lines, .. } => Event::Motd(Motd { lines }),
        papo_core::Event::Line(message) => {
            if message.as_ref().command_is("001") {
                return CoreEvent::Unknown;
            }
            return CoreEvent::Line(CoreLine::Complete(message));
        }
        papo_core::Event::MultilineMessage(message) => {
            return CoreEvent::Line(CoreLine::Complete(message.message));
        }
        papo_core::Event::Incomplete(message) => {
            return CoreEvent::Line(CoreLine::Incomplete(message.message));
        }
        papo_core::Event::TypingChanged(change) => {
            actor_token = Some(change.__actor_token());
            let change_peer = change.__peer_token();
            peer_token = Some(change_peer);
            let return_address = change.return_address();
            let reply_token = private_reply_token(&change.conversation, change_peer);
            let conversation = conversation_handle(change.conversation, change_peer);
            let envelope = EventEnvelope {
                msgid: None,
                server_time: change.server_time,
                received_at,
                account: Account::Unknown,
            };
            let Some(actor) = handle_from_raw(&change.actor, &envelope) else {
                return CoreEvent::Unknown;
            };
            let actor_party = Party::from_core(change.actor_party);
            let receipt = crate::message_ref::Receipt::new(
                Source::User(actor.clone()),
                &conversation,
                reply_token,
                return_address,
            );
            Event::TypingChanged(TypingChanged {
                server_time: change.server_time,
                received_at,
                actor,
                actor_party,
                conversation,
                status: change.status,
                receipt,
                state: change.state,
                reason: change.reason,
            })
        }
        papo_core::Event::TagMessage(message) => {
            actor_token = Some(message.actor_token);
            peer_token = Some(message.peer_token);
            let return_address = message.__return_address().to_owned();
            let reply_token = private_reply_token(&message.conversation, message.peer_token);
            let envelope = map_envelope(&message.envelope);
            let tags = Tags::from_pairs(message.tags);
            let Some(actor) = handle_from_raw(&message.actor, &envelope) else {
                return CoreEvent::Unknown;
            };
            let actor_party = Party::from_core(message.actor_party);
            let conversation = conversation_handle(message.conversation, message.peer_token);
            let mut receipt = crate::message_ref::Receipt::new(
                Source::User(actor.clone()),
                &conversation,
                reply_token,
                return_address,
            );
            receipt.set_reference(envelope.msgid.iter().cloned(), None);
            let tag = TagMsg {
                msgid: envelope.msgid,
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                actor,
                actor_party,
                conversation,
                status: message.status,
                receipt,
                tags,
            };
            let reply_to = tag
                .tags
                .get("+reply")
                .or_else(|| tag.tags.get("+draft/reply"));
            let reaction = tag
                .tags
                .get("+draft/react")
                .map(|reaction| (reaction, false))
                .or_else(|| {
                    tag.tags
                        .get("+draft/unreact")
                        .map(|reaction| (reaction, true))
                });
            if let (Some((reaction, removed)), Some(msgid)) = (reaction, reply_to) {
                Event::Reacted(Reacted {
                    msgid: tag.msgid,
                    server_time: tag.server_time,
                    received_at: tag.received_at,
                    actor: tag.actor,
                    actor_party: tag.actor_party,
                    conversation: tag.conversation,
                    status: tag.status,
                    receipt: tag.receipt,
                    reacts_to: msgid.into(),
                    reaction: reaction.to_owned(),
                    action: if removed {
                        ReactionAction::Removed
                    } else {
                        ReactionAction::Added
                    },
                })
            } else if let Some(state) = tag.tags.get("+typing").and_then(|state| match state {
                "active" => Some(crate::Typing::Active),
                "paused" => Some(crate::Typing::Paused),
                "done" => Some(crate::Typing::Done),
                _ => None,
            }) {
                let mut receipt = tag.receipt;
                receipt.set_reference([], None);
                Event::TypingChanged(TypingChanged {
                    server_time: tag.server_time,
                    received_at: tag.received_at,
                    actor: tag.actor,
                    actor_party: tag.actor_party,
                    conversation: tag.conversation,
                    status: tag.status,
                    receipt,
                    state,
                    reason: TypingReason::Notified,
                })
            } else {
                Event::TagMsg(tag)
            }
        }
        papo_core::Event::Redacted(redacted) => {
            actor_token = Some(redacted.actor_token);
            peer_token = Some(redacted.peer_token);
            let return_address = redacted.__return_address().to_owned();
            let reply_token = private_reply_token(&redacted.conversation, redacted.peer_token);
            let envelope = map_envelope(&redacted.envelope);
            let Some(actor) = actor_from_raw(&redacted.actor, &envelope) else {
                return CoreEvent::Unknown;
            };
            let actor_party = Party::from_core(redacted.actor_party);
            let conversation = conversation_handle(redacted.conversation, redacted.peer_token);
            let mut receipt = crate::message_ref::Receipt::new(
                actor.clone(),
                &conversation,
                reply_token,
                return_address,
            );
            receipt.set_reference(envelope.msgid.iter().cloned(), None);
            Event::Redacted(Redacted {
                msgid: envelope.msgid,
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                actor,
                actor_party,
                conversation,
                status: redacted.status,
                receipt,
                redacts: redacted.msgid.into(),
                reason: redacted
                    .reason
                    .filter(|reason| !reason.is_empty())
                    .map(|reason| papo_proto::display_text(&reason).into_owned()),
            })
        }
        papo_core::Event::Response(response) => return CoreEvent::Response(response),
        papo_core::Event::RequestUntracked { id, .. } => return CoreEvent::RequestUntracked(id),
        papo_core::Event::RequestTimedOut { id, .. } => return CoreEvent::RequestTimedOut(id),
        papo_core::Event::RequestFailed(id, error) => {
            return CoreEvent::RequestFailed { id, error };
        }
        papo_core::Event::RequestAborted {
            id, transmitted, ..
        } => {
            return CoreEvent::RequestAborted { id, transmitted };
        }
        papo_core::Event::TransmitAborted { token, .. } => {
            return CoreEvent::TransmitAborted(token);
        }
        papo_core::Event::RequestRecipientGone { id, .. } => {
            return CoreEvent::RequestRecipientGone(id);
        }
        papo_core::Event::TransmitRecipientGone { token, .. } => {
            return CoreEvent::TransmitRecipientGone(token);
        }
        papo_core::Event::RequestCredentialsRefused { id, .. } => {
            return CoreEvent::RequestCredentialsRefused(id);
        }
        papo_core::Event::TransmitCredentialsRefused { token, .. } => {
            return CoreEvent::TransmitCredentialsRefused(token);
        }
        papo_core::Event::RequestUnsupported {
            id, unsupported, ..
        } => {
            return CoreEvent::RequestUnsupported { id, unsupported };
        }
        papo_core::Event::StandardReply(reply) => Event::StandardReply(StandardReply {
            kind: match reply.kind {
                papo_proto::StandardReplyKind::Fail => StandardReplyKind::Fail,
                papo_proto::StandardReplyKind::Warn => StandardReplyKind::Warn,
                papo_proto::StandardReplyKind::Note => StandardReplyKind::Note,
                _ => return CoreEvent::Unknown,
            },
            command: reply.command.into(),
            code: reply.code.into(),
            context: reply.context,
            description: reply.description,
        }),
        papo_core::Event::AccountCreated { account, .. } => {
            Event::AccountCreation(crate::AccountOutcome::Created { account })
        }
        papo_core::Event::VerificationRequired {
            account, message, ..
        } => {
            Event::AccountCreation(crate::AccountOutcome::VerificationRequired { account, message })
        }
        #[cfg(feature = "unstable")]
        papo_core::Event::PersistenceStatus(status) => Event::PersistenceChanged(status),
        #[cfg(feature = "unstable")]
        papo_core::Event::BouncerNetworkChanged { id, network, .. } => {
            Event::BouncerNetworkChanged(BouncerNetworkChanged { id, network })
        }
        papo_core::Event::MetadataChanged {
            subject,
            subject_token,
            key,
            value,
            ..
        } => Event::MetadataChanged(MetadataChanged {
            subject: match subject {
                papo_core::Subject::Channel(channel) => {
                    crate::Subject::Channel(channel_handle(channel))
                }
                papo_core::Subject::User(nick) => {
                    crate::Subject::User(user_at_arrival(nick, Account::Unknown, subject_token))
                }
                _ => return CoreEvent::Unknown,
            },
            key: crate::MetadataKey::new(key),
            value,
        }),
        papo_core::Event::History {
            conversation,
            items,
            end,
            ..
        } => {
            return CoreEvent::History {
                conversation,
                items,
                end,
            };
        }
        papo_core::Event::ReadMarkerChanged(marker) => {
            Event::ReadMarkerChanged(ReadMarkerChanged {
                conversation: conversation_handle(marker.conversation, marker.peer_token),
                marker: crate::ReadMarker::reported(marker.read_until),
            })
        }
        papo_core::Event::BatchAbandoned {
            reference,
            batch_type,
            reason,
            ..
        } => Event::Warning(Warning::BatchAbandoned {
            id: reference,
            batch_type,
            reason,
        }),
        papo_core::Event::Malformed { line, .. } => Event::Warning(Warning::Malformed { line }),
        papo_core::Event::Close { kind, reason, .. } => return CoreEvent::Close { kind, reason },
        papo_core::Event::AuthenticationFailed { reason, .. } => {
            Event::Warning(Warning::AuthenticationFailed {
                reason: papo_proto::display_text(&reason).into_owned(),
            })
        }
        papo_core::Event::CapsChanged { added, removed, .. } => {
            Event::CapsChanged(CapsChanged { added, removed })
        }
        papo_core::Event::FeaturesChanged { added, removed, .. } => {
            Event::FeaturesChanged(FeaturesChanged { added, removed })
        }
        papo_core::Event::CasemappingChanged { old, new, .. } => {
            Event::CasemappingChanged(CasemappingChanged { old, new })
        }
        papo_core::Event::ChannelRecreated {
            channel,
            previous_created_at,
            created_at,
            ..
        } => Event::Warning(Warning::ChannelRecreated {
            channel: channel_handle(channel),
            previous_created_at,
            created_at,
        }),
        papo_core::Event::Utf8CasemappingUnsupported => {
            Event::Warning(Warning::Utf8CasemappingUnsupported)
        }
        papo_core::Event::StsUpgrade { port, .. } => return CoreEvent::StsUpgrade(port),
        papo_core::Event::StsPolicy(action) => return CoreEvent::StsPolicy(action),
        papo_core::Event::Joined {
            envelope,
            actor: _,
            realname: _,
            channel,
            nick,
            token,
            actor_party,
            netjoin,
            ..
        } => {
            let envelope = map_envelope(&envelope);
            let actor = user_at_arrival(nick, envelope.account.clone(), token);
            Event::Joined(Joined {
                msgid: envelope.msgid,
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                channel: channel_handle(channel),
                actor,
                actor_party: Party::from_core(actor_party),
                netjoin: netjoin.map(SplitServers::from_core),
            })
        }
        papo_core::Event::ChannelSynced { channel, .. } => Event::ChannelSynced(ChannelSynced {
            channel: channel_handle(channel),
        }),
        papo_core::Event::MembersRefreshed { channel, .. } => {
            Event::MembersRefreshed(MembersRefreshed {
                channel: channel_handle(channel),
            })
        }
        papo_core::Event::Parted {
            envelope,
            nick,
            token,
            facts,
            channel,
            reason,
            actor_party,
            ..
        } => {
            let envelope = map_envelope(&envelope);
            let account = facts
                .as_ref()
                .map_or_else(|| envelope.account.clone(), |user| user.account().clone());
            let actor = user_at_arrival(nick, account, token);
            Event::Parted(Parted {
                msgid: envelope.msgid,
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                actor,
                facts,
                channel: channel_handle(channel),
                reason: reason.map(|reason| papo_proto::display_text(&reason).into_owned()),
                actor_party: Party::from_core(actor_party),
            })
        }
        papo_core::Event::Kicked {
            envelope,
            channel,
            subject,
            token,
            facts,
            actor,
            actor_token,
            actor_party,
            subject_party,
            reason,
            ..
        } => {
            let envelope = map_envelope(&envelope);
            let Some(actor) = source::from_line(
                source::LinePrefix::Core(actor),
                &envelope,
                casemapping,
                origin,
                prove_not_bot,
            ) else {
                return CoreEvent::Unknown;
            };
            let actor = source_at_arrival(actor, actor_token);
            Event::Kicked(Kicked {
                msgid: envelope.msgid,
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                channel: channel_handle(channel),
                subject: user_at_arrival(
                    subject,
                    facts
                        .as_ref()
                        .map_or(crate::Account::Unknown, |user| user.account().clone()),
                    token,
                ),
                facts,
                actor,
                actor_party: Party::from_core(actor_party),
                subject_party: Party::from_core(subject_party),
                reason: reason.map(|reason| papo_proto::display_text(&reason).into_owned()),
            })
        }
        papo_core::Event::Invited {
            envelope,
            channel,
            subject,
            token,
            actor,
            actor_token,
            actor_party,
            subject_party,
            ..
        } => {
            let envelope = map_envelope(&envelope);
            let Some(actor) = actor_from_raw(&actor, &envelope) else {
                return CoreEvent::Unknown;
            };
            let actor = source_at_arrival(actor, actor_token);
            let reply_token = actor.user().map(|_| actor_token);
            let receipt = crate::message_ref::Receipt::without_recipient(
                actor.clone(),
                reply_token,
                actor.to_string(),
                origin,
            );
            Event::Invited(Invited {
                msgid: envelope.msgid,
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                channel: channel_handle(channel),
                subject: user_at_arrival(subject, Account::Unknown, token),
                actor,
                actor_party: Party::from_core(actor_party),
                subject_party: Party::from_core(subject_party),
                receipt,
            })
        }
        papo_core::Event::Quit {
            envelope,
            actor: _,
            nick,
            token,
            facts,
            reason,
            channels,
            actor_party,
            netsplit,
            ..
        } => {
            let envelope = map_envelope(&envelope);
            let account = facts
                .as_ref()
                .map_or_else(|| envelope.account.clone(), |user| user.account().clone());
            let actor = user_at_arrival(nick, account, token);
            Event::Quit(Quit {
                msgid: envelope.msgid,
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                actor,
                facts,
                reason: reason.map(|reason| papo_proto::display_text(&reason).into_owned()),
                channels: channels.into_iter().map(&channel_handle).collect(),
                actor_party: Party::from_core(actor_party),
                netsplit: netsplit.map(SplitServers::from_core),
            })
        }
        papo_core::Event::NickChanged {
            envelope,
            actor: _,
            old,
            new,
            token,
            channels,
            actor_party,
            ..
        } => {
            let envelope = map_envelope(&envelope);
            let actor = user_at_arrival(new.clone(), envelope.account.clone(), token);
            Event::NickChanged(NickChanged {
                msgid: envelope.msgid,
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                actor,
                old,
                new,
                channels: channels.into_iter().map(&channel_handle).collect(),
                actor_party: Party::from_core(actor_party),
            })
        }
        papo_core::Event::ChannelRenamed {
            envelope,
            old,
            new,
            actor,
            actor_token,
            actor_party,
            reason,
            ..
        } => {
            let envelope = map_envelope(&envelope);
            let Some(actor) = actor_from_raw(&actor, &envelope) else {
                return CoreEvent::Unknown;
            };
            let actor = source_at_arrival(actor, actor_token);
            Event::ChannelRenamed(ChannelRenamed {
                msgid: envelope.msgid,
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                channel: channel_handle(new.clone()),
                old,
                new,
                actor,
                actor_party: Party::from_core(actor_party),
                reason: reason.map(|reason| papo_proto::display_text(&reason).into_owned()),
            })
        }
        papo_core::Event::AccountChanged {
            envelope,
            nick,
            account,
            token,
            actor_party,
            ..
        } => {
            let envelope = map_envelope(&envelope);
            let actor = user_at_arrival(nick, account.clone(), token);
            Event::AccountChanged(AccountChanged {
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                actor,
                new: account,
                actor_party: Party::from_core(actor_party),
            })
        }
        papo_core::Event::AwayChanged {
            envelope,
            nick,
            away,
            token,
            actor_party,
            ..
        } => {
            let envelope = map_envelope(&envelope);
            let actor = user_at_arrival(nick, envelope.account.clone(), token);
            Event::AwayChanged(AwayChanged {
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                actor,
                new: away,
                actor_party: Party::from_core(actor_party),
            })
        }
        papo_core::Event::HostChanged {
            envelope,
            nick,
            username,
            host,
            token,
            actor_party,
            ..
        } => {
            let envelope = map_envelope(&envelope);
            let actor = user_at_arrival(nick, envelope.account.clone(), token);
            Event::HostChanged(HostChanged {
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                actor,
                new_username: username,
                new_host: host,
                actor_party: Party::from_core(actor_party),
            })
        }
        papo_core::Event::RealnameChanged {
            envelope,
            nick,
            realname,
            token,
            actor_party,
            ..
        } => {
            let envelope = map_envelope(&envelope);
            let actor = user_at_arrival(nick, envelope.account.clone(), token);
            Event::RealnameChanged(RealnameChanged {
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                actor,
                new: realname,
                actor_party: Party::from_core(actor_party),
            })
        }
        papo_core::Event::ModeChanged {
            envelope,
            subject,
            subject_token,
            actor,
            actor_token,
            actor_party,
            own_nick,
            changes,
            ..
        } => {
            let envelope = map_envelope(&envelope);
            let Some(actor) = source::from_line(
                source::LinePrefix::Core(actor),
                &envelope,
                casemapping,
                origin,
                prove_not_bot,
            ) else {
                return CoreEvent::Unknown;
            };
            let actor = source_at_arrival(actor, actor_token);
            Event::ModeChanged(ModeChanged {
                msgid: envelope.msgid,
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                subject: match subject {
                    papo_core::Subject::Channel(channel) => {
                        crate::Subject::Channel(channel_handle(channel))
                    }
                    papo_core::Subject::User(nick) => {
                        crate::Subject::User(user_at_arrival(nick, Account::Unknown, subject_token))
                    }
                    _ => return CoreEvent::Unknown,
                },
                actor,
                actor_party: Party::from_core(actor_party),
                own_nick,
                changes: changes
                    .into_iter()
                    .map(|change| {
                        ModeChange::from_core(change, |nick, token| {
                            user_at_arrival(nick, Account::Unknown, token)
                        })
                    })
                    .collect(),
            })
        }
        papo_core::Event::TopicChanged {
            envelope,
            channel,
            topic,
            actor,
            actor_token,
            actor_party,
            ..
        } => {
            let envelope = map_envelope(&envelope);
            let Some(actor) = source::from_line(
                source::LinePrefix::Core(actor),
                &envelope,
                casemapping,
                origin,
                prove_not_bot,
            ) else {
                return CoreEvent::Unknown;
            };
            let actor = source_at_arrival(actor, actor_token);
            Event::TopicChanged(TopicChanged {
                msgid: envelope.msgid,
                server_time: envelope.server_time,
                received_at: envelope.received_at,
                channel: channel_handle(channel),
                topic: topic.map(|topic| papo_proto::display_text(&topic).into_owned()),
                actor,
                actor_party: Party::from_core(actor_party),
            })
        }
        papo_core::Event::StateLimitReached { limit, .. } => {
            Event::Warning(Warning::StateLimitReached { kind: limit })
        }
        papo_core::Event::MonitorOnline { nick, token, .. } => {
            let subject = user_at_arrival(nick, Account::Unknown, token);
            let receipt = crate::message_ref::Receipt::without_recipient(
                Source::User(subject.clone()),
                Some(token),
                subject.nick().to_string(),
                origin,
            );
            Event::MonitorOnline(MonitorOnline { subject, receipt })
        }
        papo_core::Event::MonitorOffline {
            nick, token, facts, ..
        } => Event::MonitorOffline(MonitorOffline {
            subject: user_at_arrival(
                nick,
                facts
                    .as_ref()
                    .map_or(crate::Account::Unknown, |user| user.account().clone()),
                token,
            ),
            facts,
        }),
        papo_core::Event::MonitorListFull { limit, nicks, .. } => {
            Event::Warning(Warning::MonitorListFull {
                maximum: limit,
                nicks,
            })
        }
        papo_core::Event::Unsupported(unsupported) => {
            Event::Warning(Warning::Unsupported(unsupported))
        }
        _ => return CoreEvent::Unknown,
    };
    attach_message_handles(
        &mut mapped,
        LineTokens {
            actor: actor_token,
            peer: peer_token,
        },
        handle_state,
        false,
    );
    CoreEvent::Forward(mapped)
}

pub(crate) fn typed_mode_change(
    action: ModeAction,
    mode: char,
    arg: Option<&str>,
    kind: papo_proto::ModeKind,
    casemapping: Casemapping,
    origin: crate::Origin,
    member: impl FnOnce(crate::Nick) -> crate::UserHandle,
) -> ModeChange {
    let arg = arg.map(|arg| match (mode, kind) {
        ('k', _) => crate::ModeArg::Key(crate::Secret::new(arg)),
        (_, papo_proto::ModeKind::Prefix(prefix)) => crate::ModeArg::Member {
            user: member(origin.__nick(casemapping, arg)),
            prefix,
        },
        (_, papo_proto::ModeKind::List) => crate::ModeArg::Mask(origin.__mask(casemapping, arg)),
        _ => crate::ModeArg::Value(arg.to_owned()),
    });
    ModeChange::new(mode, action, arg)
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::{
        sync::Arc,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use super::{
        Audience, Broadcast, BroadcastCtcp, BroadcastTagMsg, Conversation, CoreEvent, Event,
        Incomplete, Message as ChatMessage, MessageId, MessageKind, MetadataChanged, ModeAction,
        ModeChange, Party, ServerNoticeKind, Source, StandardReplyKind, Tags, Warning, ctcp_parts,
        from_core as from_core_with_mapping, historical_message_event, message_event_from_line,
    };
    use crate::{ModeArg, Subject, Visibility};
    use papo_core::{
        Account, Away, BatchAbandonReason, Config, Connection, Feature, Session,
        StandardReply as CoreStandardReply, StateLimit, sts::ConnectionSecurity,
    };
    use papo_proto::{Casemapping, Isupport, Line, LineRef, Typing};
    use tokio::io::AsyncWriteExt as _;

    #[test]
    fn event_size_remains_bounded() {
        let size = std::mem::size_of::<Event>();
        assert!(size <= 640, "Event is {size} bytes");
    }

    fn from_core(event: papo_core::Event) -> CoreEvent {
        from_core_with_mapping(
            event,
            Casemapping::default(),
            crate::Origin::NONE,
            false,
            false,
            UNIX_EPOCH,
        )
    }

    fn map_conversation(value: papo_core::Conversation, origin: crate::Origin) -> Conversation {
        Conversation::from_core(value, origin, None, None)
    }

    fn channel(value: crate::ChannelName) -> crate::ChannelHandle {
        crate::ChannelHandle::from_parts(value, None, None)
    }

    fn channel_handle(value: crate::ChannelName) -> crate::ChannelHandle {
        crate::ChannelHandle::from_parts(value, None, None)
    }

    fn user_handle(value: crate::Nick) -> crate::UserHandle {
        crate::UserHandle::from_nick(value)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn credential_refusals_produce_typed_request_and_transmit_errors() {
        use crate::driver::test_support::{complete_connect_labelled, config, scripted, within};

        let settings = config()
            .allow_insecure_auth(false)
            .wanted_caps(["labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        complete_connect_labelled(&mut server, &mut events).await;

        let request_error = within(
            client.request_raw(
                Line::new("OPER")
                    .param("admin")
                    .param(crate::Secret::new("request-password-3751")),
            ),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            request_error,
            crate::Error::InvalidInput(crate::InvalidInput::InsecureAuthentication)
        ));

        let transmit_error = within(
            client.send_raw(Line::new("PASS").param(crate::Secret::new("transmit-password-6824"))),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            transmit_error,
            crate::Error::InvalidInput(crate::InvalidInput::InsecureAuthentication)
        ));

        within(server.shutdown()).await.unwrap();
        assert!(within(driver).await.unwrap().is_err());
    }

    fn assert_event_times(
        event: &Event,
        expected_variant: &str,
        server_time: crate::ServerTime,
        received_at: SystemTime,
    ) {
        assert_eq!(event.server_time(), Some(server_time), "{expected_variant}");
        assert_eq!(event.received_at(), Some(received_at), "{expected_variant}");
    }

    #[test]
    fn broadcast_payloads_report_the_client_origin() {
        let origin = crate::Origin::__new(73);
        let mapping = Casemapping::default();
        let line = Line::new("TAGMSG")
            .param("*")
            .with_source("oper!staff@staff.example");
        let envelope = super::line_envelope_with_account_tag(
            &line.as_ref(),
            mapping,
            origin,
            false,
            UNIX_EPOCH,
        );
        let Some(Source::User(actor)) = super::source::from_line(
            super::source::LinePrefix::Raw("oper!staff@staff.example"),
            &envelope,
            mapping,
            origin,
            false,
        ) else {
            panic!("expected user source")
        };
        for to in [
            Broadcast::Everyone,
            Broadcast::Mask(origin.__mask(mapping, "$*.example.org")),
        ] {
            let event = BroadcastTagMsg {
                actor: actor.clone(),
                actor_party: Party::Other,
                to,
                tags: Tags::default(),
                msgid: None,
                received_at: UNIX_EPOCH,
            };
            assert_eq!(event.origin(), origin);
        }
        let ctcp = BroadcastCtcp {
            actor,
            actor_party: Party::Other,
            to: Broadcast::Mask(origin.__mask(mapping, "$*.example.org")),
            command: "VERSION".into(),
            params: None,
            msgid: None,
            server_time: None,
            received_at: UNIX_EPOCH,
            tags: Tags::default(),
        };
        assert_eq!(ctcp.origin(), origin);
    }

    #[test]
    fn broadcast_ctcp_requires_a_broadcast_target() {
        let parse = |line| {
            super::broadcast_from_line(
                LineRef::parse(line).unwrap().to_owned(),
                Party::Other,
                crate::Origin::NONE,
                &Isupport::default(),
                false,
                false,
                UNIX_EPOCH,
            )
        };

        let masked = parse(":oper!o@staff PRIVMSG $*.example.org :\x01VERSION\x01").unwrap();
        assert!(matches!(masked, Event::BroadcastCtcp(event)
            if matches!(event.to, Broadcast::Mask(ref mask) if mask == "$*.example.org")));

        let everyone = parse(":oper!o@staff PRIVMSG * :\x01VERSION\x01").unwrap();
        assert!(matches!(everyone, Event::BroadcastCtcp(event)
            if event.to == Broadcast::Everyone));

        let ordinary = parse(":oper!o@staff PRIVMSG papo :\x01VERSION\x01").unwrap_err();
        assert_eq!(ordinary.as_ref().param(0), Some("papo"));
    }

    const MESSAGE_EVENT_METADATA_CASES: [(&str, &str, &str); 21] = [
        (
            "Message",
            "RfcBTzyxY120113",
            "@msgid=RfcBTzyxY120113;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example PRIVMSG #papo :hello from Ergo",
        ),
        (
            "Message",
            "RfcBTzyxY120114",
            "@msgid=RfcBTzyxY120114;time=2026-02-03T04:05:06.789Z :papo!~papo@cloak.example PRIVMSG #papo :hello from another device",
        ),
        (
            "Notice",
            "RfcBTzyxY120115",
            "@msgid=RfcBTzyxY120115;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example NOTICE #papo :maintenance begins soon",
        ),
        (
            "ServerNotice",
            "RfcBTzyxY120116",
            "@msgid=RfcBTzyxY120116;time=2026-02-03T04:05:06.789Z :irc.ergo.chat NOTICE papo :This server is in maintenance mode",
        ),
        (
            "CtcpQuery",
            "RfcBTzyxY120101",
            "@msgid=RfcBTzyxY120101;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example PRIVMSG #papo :\x01VERSION\x01",
        ),
        (
            "CtcpReply",
            "RfcBTzyxY120117",
            "@msgid=RfcBTzyxY120117;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example NOTICE #papo :\x01VERSION Ergo-2.15.0\x01",
        ),
        (
            "BroadcastCtcp",
            "RfcBTzyxY120118",
            "@msgid=RfcBTzyxY120118;time=2026-02-03T04:05:06.789Z :oper!staff@staff.example PRIVMSG $*.example.org :\x01VERSION\x01",
        ),
        (
            "BroadcastTagMsg",
            "RfcBTzyxY120119",
            "@msgid=RfcBTzyxY120119;time=2026-02-03T04:05:06.789Z;+typing=active;+draft/react=👍;+reply=RfcBTzyxY120001 :oper!staff@staff.example TAGMSG *",
        ),
        (
            "TagMsg",
            "RfcBTzyxY120102",
            "@msgid=RfcBTzyxY120102;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example TAGMSG #papo",
        ),
        (
            "Reacted",
            "RfcBTzyxY120103",
            "@msgid=RfcBTzyxY120103;time=2026-02-03T04:05:06.789Z;+draft/react=👍;+reply=RfcBTzyxY120001 :emily!~emily@cloak.example TAGMSG #papo",
        ),
        (
            "Redacted",
            "RfcBTzyxY120104",
            "@msgid=RfcBTzyxY120104;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example REDACT #papo RfcBTzyxY120001 :off-topic",
        ),
        (
            "Joined",
            "RfcBTzyxY120105",
            "@msgid=RfcBTzyxY120105;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example JOIN #papo",
        ),
        (
            "Parted",
            "RfcBTzyxY120106",
            "@msgid=RfcBTzyxY120106;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example PART #papo :Lunch",
        ),
        (
            "Kicked",
            "RfcBTzyxY120107",
            "@msgid=RfcBTzyxY120107;time=2026-02-03T04:05:06.789Z :alice!~alice@cloak.example KICK #papo emily :Flooding",
        ),
        (
            "Invited",
            "RfcBTzyxY120120",
            "@msgid=RfcBTzyxY120120;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example INVITE papo #papo",
        ),
        (
            "Quit",
            "RfcBTzyxY120108",
            "@msgid=RfcBTzyxY120108;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example QUIT :Client Quit",
        ),
        (
            "NickChanged",
            "RfcBTzyxY120109",
            "@msgid=RfcBTzyxY120109;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example NICK emily_away",
        ),
        (
            "ChannelRenamed",
            "RfcBTzyxY120121",
            "@msgid=RfcBTzyxY120121;time=2026-02-03T04:05:06.789Z :alice!~alice@cloak.example RENAME #papo #papo-dev :Release complete",
        ),
        (
            "ModeChanged",
            "RfcBTzyxY120110",
            "@msgid=RfcBTzyxY120110;time=2026-02-03T04:05:06.789Z :alice!~alice@cloak.example MODE #papo +o emily",
        ),
        (
            "TopicChanged",
            "RfcBTzyxY120111",
            "@msgid=RfcBTzyxY120111;time=2026-02-03T04:05:06.789Z :alice!~alice@cloak.example TOPIC #papo :Release planning",
        ),
        (
            "Other",
            "RfcBTzyxY120112",
            "@msgid=RfcBTzyxY120112;time=2026-02-03T04:05:06.789Z :irc.example 999 papo :Vendor-specific reply",
        ),
    ];

    fn event(
        message: Line,
        isupport: &Isupport,
        nick: &str,
        received_at: std::time::SystemTime,
    ) -> ChatMessage {
        ChatMessage::new(message, isupport, nick, received_at).unwrap()
    }

    fn parse_event(
        message: Line,
        isupport: &Isupport,
        nick: &str,
        received_at: std::time::SystemTime,
    ) -> Event {
        let (status, conversation) =
            papo_core::classify_conversation(&message.as_ref(), nick, isupport).unwrap_or((
                None,
                papo_core::Conversation::Private(isupport.casemapping().nick(nick)),
            ));
        let conversation = map_conversation(conversation, crate::Origin::NONE);
        message_event_from_line(
            message,
            None,
            Party::Other,
            conversation,
            status,
            None,
            isupport,
            false,
            false,
            received_at,
            None,
        )
    }

    fn mapped_live_event(line: &str, now: Instant) -> Event {
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(now, line.as_bytes());
        let core = session.poll_event().expect("line produced an event");
        match from_core_with_mapping(
            core,
            session.isupport().casemapping(),
            crate::Origin::NONE,
            false,
            false,
            UNIX_EPOCH,
        ) {
            CoreEvent::Forward(event) => event,
            CoreEvent::Line(super::CoreLine::Complete(line)) => super::incoming_from_core(
                line,
                crate::Origin::NONE,
                session.isupport(),
                false,
                false,
                UNIX_EPOCH,
                None,
            ),
            _ => panic!("line did not produce a public event"),
        }
    }

    #[test]
    fn a_restricted_line_files_under_its_plain_channel() {
        let actor = crate::__testing::user_handle(Casemapping::default().nick("oppy"));
        let message = crate::__testing::message(Some(actor), "#ops", "deploy key rotated?", false);
        let event = Event::Restricted(crate::Restricted {
            audience: crate::Prefix::__new('o', '@'),
            line: crate::RestrictedLine::Message(message.clone()),
        });

        assert_eq!(event.conversation().as_ref(), Some(message.conversation()));
        let restricted = event.as_restricted().expect("restricted arm");
        assert_eq!(restricted.audience.symbol(), '@');
        assert_eq!(restricted.line.as_message(), Some(&message));
    }

    #[test]
    fn ergo_and_soju_wire_times_round_trip_through_stored_forms() {
        for line in [
            "@time=2026-11-08T19:55:04.123Z :alice!a@cloak PRIVMSG #papo :from Ergo",
            "@time=2026-11-08T19:55:04.123Z :bob!b@soju NOTICE #papo :from soju",
        ] {
            let parsed = mapped_live_event(line, Instant::now())
                .server_time()
                .unwrap();
            let text = parsed.to_string();
            assert_eq!(text, "2026-11-08T19:55:04.123Z");
            assert_eq!(crate::ServerTime::from_stored(&text), Ok(parsed));
        }
    }

    #[test]
    fn incomplete_message_has_no_return_address() {
        let now = Instant::now();
        let cases = [
            (
                "Message",
                "@msgid=RfcBTzyxY120301;time=2026-02-03T04:05:06.789Z :alice!~a@cloak.example PRIVMSG #review :deployment in progress",
            ),
            (
                "Notice",
                "@msgid=RfcBTzyxY120302;time=2026-02-03T04:05:06.789Z :alice!~a@cloak.example NOTICE #review :maintenance in progress",
            ),
        ];

        for (expected, line) in cases {
            let event = match mapped_live_event(line, now) {
                Event::Message(message) => Event::Incomplete(Incomplete::Message(message)),
                Event::Notice(notice) => Event::Incomplete(Incomplete::Notice(notice)),
                other => panic!("unexpected mapped event: {other:?}"),
            };
            let Event::Incomplete(incomplete) = &event else {
                panic!("expected incomplete {expected}")
            };
            match (expected, incomplete) {
                ("Message", Incomplete::Message(message)) => {
                    assert_eq!(message.text, "deployment in progress");
                    assert!(incomplete.as_message().is_some());
                    assert!(incomplete.as_notice().is_none());
                }
                ("Notice", Incomplete::Notice(notice)) => {
                    assert_eq!(notice.text, "maintenance in progress");
                    assert!(incomplete.as_notice().is_some());
                    assert!(incomplete.as_message().is_none());
                }
                _ => panic!("wrong incomplete variant: {incomplete:?}"),
            }
            assert_eq!(
                event
                    .conversation()
                    .map(|conversation| conversation.to_string()),
                Some("#review".to_owned())
            );
            assert!(event.msgid().is_some());
            assert!(event.server_time().is_some());
            assert_eq!(event.received_at(), Some(UNIX_EPOCH));
            let class = event.overflow_class();
            assert_eq!(class.priority, super::OverflowPriority::Message);
            assert_eq!(
                class.conversation.map(ToString::to_string).as_deref(),
                Some("#review")
            );
        }
    }

    fn has_channel_guard(expected: &str, event: &Event) -> bool {
        let message =
            |address: crate::MessageRef| crate::Target::__parts(&address).channel.is_some();
        let notice = |address: crate::NoticeRef| crate::Target::__parts(&address).channel.is_some();
        match (expected, event) {
            ("CtcpQuery", Event::CtcpQuery(event)) => message(event.return_address()),
            ("CtcpReply", Event::CtcpReply(event)) => notice(event.return_address()),
            ("Message", Event::Message(event)) => message(event.return_address()),
            ("Notice", Event::Notice(event)) => notice(event.return_address()),
            ("ServerNotice", Event::ServerNotice(event)) => {
                event.return_address().is_some_and(notice)
            }
            ("Reacted", Event::Reacted(event)) => message(event.return_address()),
            ("Redacted", Event::Redacted(event)) => message(event.return_address()),
            ("TagMsg", Event::TagMsg(event)) => message(event.return_address()),
            ("TypingChanged", Event::TypingChanged(event)) => message(event.return_address()),
            (_, event) => panic!("expected {expected}, got {event:?}"),
        }
    }

    #[test]
    fn every_message_event_variant_carries_its_channel_guard() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(now, b":papo!p@localhost JOIN #review");
        while session.poll_event().is_some() {}
        let shared = Arc::new(std::sync::RwLock::new(Arc::new(
            crate::State::from_session(&session, true),
        )));
        let cases = [
            (
                "CtcpQuery",
                ":alice!~alice@cloak.example PRIVMSG #review :\x01VERSION\x01",
            ),
            (
                "CtcpReply",
                ":alice!~alice@cloak.example NOTICE #review :\x01VERSION Ergo-2.15.0\x01",
            ),
            (
                "Message",
                ":alice!~alice@cloak.example PRIVMSG #review :release approved",
            ),
            (
                "Notice",
                ":alice!~alice@cloak.example NOTICE #review :maintenance begins soon",
            ),
            (
                "ServerNotice",
                ":ergo.test NOTICE #review :Network maintenance begins soon",
            ),
            (
                "Reacted",
                "@+draft/react=👍;+draft/reply=RfcBTzyxY120001 :alice!~alice@cloak.example TAGMSG #review",
            ),
            (
                "Redacted",
                ":alice!~alice@cloak.example REDACT #review RfcBTzyxY120001 :off-topic",
            ),
            (
                "TagMsg",
                "@+example/status=ready :alice!~alice@cloak.example TAGMSG #review",
            ),
            (
                "TypingChanged",
                "@+typing=active :alice!~alice@cloak.example TAGMSG #review",
            ),
        ];

        for (expected, line) in cases {
            session.handle_line(now, line.as_bytes());
            let core = session.poll_event().expect("line produced an event");
            *shared.write().unwrap() = Arc::new(crate::State::from_session(&session, true));
            let event = match super::from_core_with_receipt(
                core,
                session.isupport().casemapping(),
                crate::Origin::NONE,
                false,
                false,
                UNIX_EPOCH,
                Some(session.state()),
                Some(&shared),
            ) {
                CoreEvent::Forward(event) => event,
                CoreEvent::Line(super::CoreLine::Complete(line)) => super::incoming_from_core(
                    line,
                    crate::Origin::NONE,
                    session.isupport(),
                    false,
                    false,
                    UNIX_EPOCH,
                    Some(&shared),
                ),
                _ => panic!("line did not produce a public event"),
            };
            assert!(has_channel_guard(expected, &event), "{expected}");
        }
    }

    #[test]
    fn core_channel_receipt_captures_the_membership_guard() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(now, b":papo!p@localhost JOIN #review");
        while session.poll_event().is_some() {}
        session.handle_line(
            now,
            b"@+example/status=ready :alice!~alice@cloak.example TAGMSG #review",
        );
        let event = super::from_core_with_receipt(
            session.poll_event().unwrap(),
            session.isupport().casemapping(),
            crate::Origin::NONE,
            false,
            false,
            UNIX_EPOCH,
            Some(session.state()),
            None,
        );
        let CoreEvent::Forward(event) = event else {
            panic!("expected tag message")
        };
        assert!(has_channel_guard("TagMsg", &event));
    }

    #[test]
    fn conversation_reply_tokens_guard_only_private_recipients() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(now, b":alice!~alice@cloak.example JOIN #review");
        let mapping = session.isupport().casemapping();
        let private =
            papo_core::Conversation::Private(crate::Origin::NONE.__nick(mapping, "alice"));
        let channel =
            papo_core::Conversation::Channel(crate::Origin::NONE.__channel(mapping, "#review"));

        let alice = session.state().__reply_token_for("alice");
        assert_eq!(super::private_reply_token(&private, alice), Some(alice));
        assert_eq!(super::private_reply_token(&channel, alice), None);
    }

    #[test]
    fn every_message_overflow_variant_keeps_its_priority_and_conversation() {
        let now = Instant::now();
        let cases = [
            (
                "Message",
                ":alice!~alice@cloak.example PRIVMSG #review :release approved",
                Some("#review"),
            ),
            (
                "Notice",
                ":alice!~alice@cloak.example NOTICE #review :maintenance begins soon",
                Some("#review"),
            ),
            (
                "CtcpQuery",
                ":alice!~alice@cloak.example PRIVMSG #review :\x01VERSION\x01",
                Some("#review"),
            ),
            (
                "CtcpReply",
                ":alice!~alice@cloak.example NOTICE #review :\x01VERSION Ergo-2.15.0\x01",
                Some("#review"),
            ),
            (
                "TagMsg",
                "@+example/status=ready :alice!~alice@cloak.example TAGMSG #review",
                Some("#review"),
            ),
            (
                "Reacted",
                "@+draft/react=👍;+draft/reply=RfcBTzyxY120001 :alice!~alice@cloak.example TAGMSG #review",
                Some("#review"),
            ),
            (
                "Redacted",
                ":alice!~alice@cloak.example REDACT #review RfcBTzyxY120001 :off-topic",
                Some("#review"),
            ),
            (
                "BroadcastCtcp",
                ":oper!staff@staff.example PRIVMSG $*.example.org :\x01VERSION\x01",
                None,
            ),
            (
                "BroadcastTagMsg",
                "@+example/status=ready :oper!staff@staff.example TAGMSG *",
                None,
            ),
        ];

        for (expected, line, conversation) in cases {
            let event = mapped_live_event(line, now);
            let variant = match &event {
                Event::Message(_) => "Message",
                Event::Notice(_) => "Notice",
                Event::CtcpQuery(_) => "CtcpQuery",
                Event::CtcpReply(_) => "CtcpReply",
                Event::TagMsg(_) => "TagMsg",
                Event::Reacted(_) => "Reacted",
                Event::Redacted(_) => "Redacted",
                Event::BroadcastCtcp(_) => "BroadcastCtcp",
                Event::BroadcastTagMsg(_) => "BroadcastTagMsg",
                event => panic!("expected {expected}, got {event:?}"),
            };
            assert_eq!(variant, expected);
            let class = event.overflow_class();
            assert_eq!(
                class.priority,
                super::OverflowPriority::Message,
                "{expected}"
            );
            assert_eq!(
                class.conversation.map(ToString::to_string).as_deref(),
                conversation,
                "{expected}"
            );
        }
    }

    #[test]
    fn state_and_other_overflow_priorities_are_distinct() {
        let now = Instant::now();
        let cases = [
            (
                "Joined",
                ":alice!~alice@cloak.example JOIN #review",
                super::OverflowPriority::State,
            ),
            (
                "ServerNotice",
                ":ergo.test NOTICE papo :*** Looking up your hostname...",
                super::OverflowPriority::Other,
            ),
        ];

        for (expected, line, priority) in cases {
            let event = mapped_live_event(line, now);
            let variant = match &event {
                Event::Joined(_) => "Joined",
                Event::ServerNotice(_) => "ServerNotice",
                event => panic!("expected {expected}, got {event:?}"),
            };
            assert_eq!(variant, expected);
            let class = event.overflow_class();
            assert_eq!(class.priority, priority, "{expected}");
            assert!(class.conversation.is_none(), "{expected}");
        }
    }

    #[test]
    fn every_message_conversation_kind_keeps_its_origin() {
        let origin = crate::Origin::__new(73);
        let mapping = Casemapping::default();
        let isupport = Isupport::default();
        let cases = [
            (
                "channel",
                Line::privmsg("#papo", "hello").with_source("emily!~e@localhost"),
                map_conversation(
                    papo_core::Conversation::Channel(origin.__channel(mapping, "#papo")),
                    origin,
                ),
            ),
            (
                "private",
                Line::notice("papo", "hello").with_source("emily!~e@localhost"),
                Conversation::Private(user_handle(origin.__nick(mapping, "emily"))),
            ),
        ];

        for (kind, line, conversation) in cases {
            let event = message_event_from_line(
                line,
                None,
                Party::Other,
                conversation,
                None,
                None,
                &isupport,
                false,
                false,
                UNIX_EPOCH,
                None,
            );
            match (kind, event) {
                ("channel", Event::Message(message)) => {
                    assert_eq!(message.origin(), origin);
                    assert_eq!(message.actor.nick().origin(), origin);
                }
                ("private", Event::Notice(notice)) => {
                    assert_eq!(notice.origin(), origin);
                    assert_eq!(notice.actor.nick().origin(), origin);
                }
                ("broadcast", Event::ServerNotice(notice)) => {
                    assert_eq!(notice.actor.nick().unwrap().origin(), origin);
                    assert!(
                        matches!(notice.to, Audience::Broadcast(Broadcast::Mask(mask))
                        if mask.origin() == origin)
                    );
                }
                (kind, event) => panic!("unexpected {kind} event: {event:?}"),
            }
        }
    }

    #[test]
    fn routes_user_and_server_notices() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CHANTYPES=#", "STATUSMSG=@+"]);
        let user = parse_event(
            Line::notice("papo", "maintenance soon")
                .with_source("emily!~e@localhost")
                .with_tag("account", "emily"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        assert!(matches!(user, Event::Notice(notice)
            if notice.kind == MessageKind::Text
                && matches!(notice.conversation, Conversation::Private(_))
                && matches!(notice.actor.account(), Account::LoggedIn(account) if account == "emily")));

        let action = parse_event(
            Line::notice("@#ops", "\x01ACTION nods hello\x01")
                .with_source("emily!~e@localhost")
                .with_tag("msgid", "RfcBTzyxY120007"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        let Event::Notice(notice) = action else {
            panic!("expected user notice")
        };
        assert_eq!(notice.kind, MessageKind::Action);
        assert_eq!(notice.text, "nods hello");
        assert_eq!(notice.return_address(), "@#ops");
        let reference = notice.return_address();
        assert_eq!(reference, "@#ops");

        let server = parse_event(
            Line::notice("papo", "Network maintenance begins soon").with_source("irc.example"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        let Event::ServerNotice(server) = server else {
            panic!("expected server notice")
        };
        assert_eq!(server.kind, ServerNoticeKind::Notice);
        assert_eq!(server.to, Audience::Me);
        assert_eq!(server.actor_party, Party::Other);
        assert!(server.status.is_none());
        assert!(server.return_address().is_none());

        let wallops = message_event_from_line(
            Line::new("WALLOPS")
                .param("Network maintenance begins soon")
                .with_source("irc.example"),
            None,
            Party::Other,
            Conversation::Private(user_handle(isupport.casemapping().nick("papo"))),
            None,
            None,
            &isupport,
            false,
            false,
            UNIX_EPOCH,
            None,
        );
        let Event::ServerNotice(wallops) = wallops else {
            panic!("expected WALLOPS server notice")
        };
        assert_eq!(wallops.kind, ServerNoticeKind::Wallops);
        assert_eq!(wallops.to, Audience::Broadcast(Broadcast::Everyone));
        assert!(wallops.return_address().is_none());

        let broadcast = message_event_from_line(
            Line::notice("$*.example.org", "Network maintenance begins soon")
                .with_source("oper!o@staff")
                .with_tag("account", "network-admin"),
            None,
            Party::Other,
            Conversation::Private(user_handle(isupport.casemapping().nick("papo"))),
            None,
            None,
            &isupport,
            true,
            false,
            UNIX_EPOCH,
            None,
        );
        let Event::ServerNotice(broadcast) = broadcast else {
            panic!("expected broadcast server notice")
        };
        assert!(matches!(
            broadcast.to,
            Audience::Broadcast(Broadcast::Mask(_))
        ));
        assert_eq!(broadcast.actor_party, Party::Other);
        assert!(
            matches!(broadcast.actor.user().map(|actor| actor.account().clone()),
            Some(Account::LoggedIn(account)) if account == "network-admin")
        );
        assert!(broadcast.return_address().is_none());
    }

    #[test]
    fn channel_server_notice_preserves_its_status_audience() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CHANTYPES=#", "STATUSMSG=@+"]);
        let event = parse_event(
            Line::notice("@#ops", "Network maintenance begins soon").with_source("irc.example"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        let Event::ServerNotice(notice) = event else {
            panic!("expected channel server notice")
        };
        assert!(matches!(notice.to, Audience::Channel(_)));
        assert_eq!(notice.return_address().as_ref().unwrap(), "@#ops");
    }

    #[test]
    fn routes_status_prefixed_server_notices() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CHANTYPES=#", "STATUSMSG=@+"]);
        for (line, channel, status) in [
            (
                ":irc.example.net NOTICE @#ops :[Knock] by mallory!~m@203.0.113.9 (let me in)",
                "#ops",
                '@',
            ),
            (
                ":irc.example NOTICE @#chan :<mallory:#chan> text",
                "#chan",
                '@',
            ),
            (
                ":irc.example NOTICE @+#ops :Operators and voiced users only",
                "#ops",
                '+',
            ),
        ] {
            let event = parse_event(
                LineRef::parse(line).unwrap().to_owned(),
                &isupport,
                "papo",
                UNIX_EPOCH,
            );
            assert!(matches!(event, Event::ServerNotice(notice)
                if matches!(notice.to, Audience::Channel(ref name) if name == channel)
                    && notice.status.map(crate::Prefix::symbol) == Some(status)));
        }
    }

    #[test]
    fn exposes_formatted_notice_spans() {
        let event = parse_event(
            LineRef::parse(
                ":emily!~emily@cloak.example NOTICE #papo :deployment \x02complete\x02 on Ergo",
            )
            .unwrap()
            .to_owned(),
            &Isupport::default(),
            "papo",
            UNIX_EPOCH,
        );
        let notice = event.as_notice().unwrap();

        let spans = notice.spans().collect::<Vec<_>>();
        assert_eq!(
            spans.iter().map(|span| span.text).collect::<Vec<_>>(),
            ["deployment ", "complete", " on Ergo"]
        );
        assert!(spans[1].style.bold);
    }

    fn chat(command: &str, text: &str) -> ChatMessage {
        event(
            Line::new(command)
                .with_params(["#papo", text])
                .with_source("emily!~emily@localhost"),
            &Isupport::default(),
            "papo",
            UNIX_EPOCH,
        )
    }

    #[test]
    fn chat_compares_with_string_references() {
        let message = event(
            LineRef::parse(":alice!a@h PRIVMSG #papo :!op")
                .unwrap()
                .to_owned(),
            &Isupport::default(),
            "papo",
            UNIX_EPOCH,
        );

        assert_eq!(message.conversation, "#Papo");
        assert_ne!(message.conversation, "#other");

        let private = event(
            Line::privmsg("papo", "release ready").with_source("[bot]!b@h"),
            &Isupport::default(),
            "papo",
            UNIX_EPOCH,
        );
        assert_eq!(private.conversation, "{bot}");
        assert_ne!(private.conversation, "[other]");
    }

    #[test]
    fn classifies_chat_and_ctcp_messages() {
        let message = chat("PRIVMSG", "hello");
        assert_eq!(message.kind, MessageKind::Text);
        assert_eq!(message.actor.account(), &Account::Unknown);
        assert_eq!(
            chat("PRIVMSG", "\x01ACTION waves\x01").kind,
            MessageKind::Action
        );
        assert_eq!(
            chat("PRIVMSG", "\x01ACTION tests a split message").kind,
            MessageKind::Action
        );
        let parse = |command, text| {
            let line = Line::new(command)
                .with_params(["@#ops", text])
                .with_source("emily!~emily@localhost")
                .with_tag("account", "emily");
            let mut isupport = Isupport::default();
            isupport.apply_tokens(["CHANTYPES=#", "STATUSMSG=@+"]);
            let (status, conversation) =
                papo_core::classify_conversation(&line.as_ref(), "papo", &isupport).unwrap();
            let conversation = map_conversation(conversation, crate::Origin::NONE);
            message_event_from_line(
                line,
                None,
                Party::Other,
                conversation,
                status,
                None,
                &isupport,
                false,
                false,
                UNIX_EPOCH,
                None,
            )
        };
        let version_event = parse("PRIVMSG", "\x01VERSION\x01");
        let version = version_event.as_ctcp_query().unwrap();
        assert_eq!(version.command, "VERSION");
        assert_eq!(version.params, None);
        assert_eq!(version.return_address(), "@#ops");
        assert!(
            matches!(version.actor.account(), Account::LoggedIn(account) if account == "emily")
        );
        let reply_event = parse("NOTICE", "\x01VERSION papo-0.1\x01");
        let reply = reply_event.as_ctcp_reply().unwrap();
        assert_eq!(reply.params.as_deref(), Some("papo-0.1"));
        assert_eq!(reply.return_address(), "@#ops");
        assert!(matches!(reply.actor.account(), Account::LoggedIn(account) if account == "emily"));
        let action_notice = parse("NOTICE", "\x01ACTION waves\x01");
        assert!(matches!(action_notice, Event::Notice(notice)
            if notice.kind == MessageKind::Action && notice.text == "waves"));
        assert!(
            ChatMessage::new(
                Line::new("FAIL").with_params(["PRIVMSG", "\x01VERSION\x01"]),
                &Isupport::default(),
                "papo",
                UNIX_EPOCH,
            )
            .is_err()
        );
        assert!(
            ChatMessage::new(
                Line::privmsg("#papo", "actorless message"),
                &Isupport::default(),
                "papo",
                UNIX_EPOCH,
            )
            .is_err()
        );
    }

    #[test]
    fn non_action_ctcp_is_not_a_message_or_notice() {
        let isupport = Isupport::default();
        let message = LineRef::parse(":emily!~emily@cloak.example PRIVMSG #papo :\x01VERSION\x01")
            .unwrap()
            .to_owned();
        assert!(ChatMessage::new(message, &isupport, "papo", UNIX_EPOCH).is_err());

        let notice =
            LineRef::parse(":emily!~emily@cloak.example NOTICE #papo :\x01VERSION Ergo-2.15.0\x01")
                .unwrap()
                .to_owned();
        let (status, conversation) =
            papo_core::classify_conversation(&notice.as_ref(), "papo", &isupport).unwrap();
        let conversation = map_conversation(conversation, crate::Origin::NONE);
        assert!(
            super::Notice::from_line(
                notice,
                None,
                Party::Other,
                conversation,
                status,
                None,
                &isupport,
                true,
                false,
                UNIX_EPOCH,
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn distinguishes_ctcp_queries_and_replies() {
        let query = Line::privmsg("#papo", "\x01VERSION\x01");
        let (_, _, reply) = ctcp_parts(&query.as_ref()).unwrap();
        assert!(!reply);

        let response = Line::new("NOTICE").with_params(["#papo", "\x01VERSION papo-0.1\x01"]);
        let (_, _, reply) = ctcp_parts(&response.as_ref()).unwrap();
        assert!(reply);

        let unrelated = Line::new("FAIL").with_params(["PRIVMSG", "\x01VERSION\x01"]);
        assert!(ctcp_parts(&unrelated.as_ref()).is_none());
    }

    #[test]
    fn exposes_raw_and_display_text_once() {
        let formatted = chat("PRIVMSG", "hello \x02from Ergo\x02");
        assert_eq!(formatted.raw_text, "hello \x02from Ergo\x02");
        assert_eq!(formatted.text, "hello from Ergo");
        let action = chat("PRIVMSG", "\x01ACTION writes the specification\x01");
        assert_eq!(action.raw_text, "writes the specification");
        assert_eq!(action.text, "writes the specification");
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one regression enumerates every exposed display-text field"
    )]
    fn sanitizes_every_server_and_peer_display_text_field() {
        let payload = "\x1b[2Jvisible";
        let parse = |command| {
            parse_event(
                LineRef::parse(&format!(
                    ":alice!~alice@cloak.example {command} #papo :{payload}"
                ))
                .unwrap()
                .to_owned(),
                &Isupport::default(),
                "papo",
                UNIX_EPOCH,
            )
        };
        let Event::Message(message) = parse("PRIVMSG") else {
            panic!("expected message")
        };
        let Event::Notice(notice) = parse("NOTICE") else {
            panic!("expected notice")
        };
        let server = parse_event(
            LineRef::parse(&format!(":ergo.test NOTICE papo :{payload}"))
                .unwrap()
                .to_owned(),
            &Isupport::default(),
            "papo",
            UNIX_EPOCH,
        );
        assert_eq!(message.text, "visible");
        assert_eq!(message.raw_text, payload);
        assert_eq!(notice.text, "visible");
        assert_eq!(notice.raw_text, payload);
        let Event::ServerNotice(server) = server else {
            panic!("expected server notice")
        };
        assert_eq!(server.text, "visible");
        assert_eq!(server.raw_text, payload);

        let Event::Parted(parted) = forwarded(papo_core::Event::__parted(
            papo_core::LineEnvelope::default(),
            Casemapping::default().channel("#papo"),
            "alice!~alice@cloak.example".into(),
            Casemapping::default().nick("alice"),
            papo_core::ReplyToken::__unknown(papo_proto::Origin::NONE),
            None,
            Some(payload.into()),
            papo_core::Party::Other,
        )) else {
            panic!("expected part")
        };
        let Event::Kicked(kicked) = forwarded(papo_core::Event::__kicked(
            papo_core::LineEnvelope::default(),
            Casemapping::default().channel("#papo"),
            Casemapping::default().nick("alice"),
            papo_core::ReplyToken::__unknown(papo_proto::Origin::NONE),
            None,
            papo_core::Source::from_proto(
                papo_proto::Source::Server("ergo.test"),
                Casemapping::default(),
            ),
            papo_core::ReplyToken::__unknown(papo_proto::Origin::NONE),
            papo_core::Party::Other,
            papo_core::Party::Other,
            Some(payload.into()),
        )) else {
            panic!("expected kick")
        };
        let Event::Quit(quit) = forwarded(papo_core::Event::__quit(
            papo_core::LineEnvelope::default(),
            "alice!~alice@cloak.example".into(),
            Casemapping::default().nick("alice"),
            papo_core::ReplyToken::__unknown(papo_proto::Origin::NONE),
            None,
            Some(payload.into()),
            vec![Casemapping::default().channel("#papo")],
            papo_core::Party::Other,
            None,
        )) else {
            panic!("expected quit")
        };
        let Event::ChannelRenamed(renamed) = forwarded(papo_core::Event::__channel_renamed(
            Casemapping::default().channel("#papo"),
            Casemapping::default().channel("#papo-chat"),
            "ergo.test".into(),
            papo_core::Party::Other,
            Some(payload.into()),
        )) else {
            panic!("expected channel rename")
        };
        let Event::TopicChanged(topic) = forwarded(papo_core::Event::__topic_changed(
            papo_core::LineEnvelope::default(),
            Casemapping::default().channel("#papo"),
            Some(payload.into()),
            papo_core::Source::from_proto(
                papo_proto::Source::Server("ergo.test"),
                Casemapping::default(),
            ),
            papo_core::ReplyToken::__unknown(crate::Origin::NONE),
            papo_core::Party::Other,
        )) else {
            panic!("expected topic")
        };
        let redacted = papo_core::Redacted {
            envelope: papo_core::LineEnvelope::default(),
            actor: "alice!~alice@cloak.example".into(),
            actor_token: papo_core::ReplyToken::__unknown(crate::Origin::NONE),
            peer_token: papo_core::ReplyToken::__unknown(crate::Origin::NONE),
            actor_party: papo_core::Party::Other,
            conversation: papo_core::Conversation::Channel(Casemapping::default().channel("#papo")),
            status: None,
            return_address: "#papo".into(),
            msgid: "RKi9Cw".into(),
            reason: Some(payload.into()),
        };
        let Event::Redacted(redacted) = forwarded(papo_core::Event::Redacted(redacted)) else {
            panic!("expected redaction")
        };

        for value in [
            parted.reason,
            kicked.reason,
            quit.reason,
            renamed.reason,
            topic.topic,
            redacted.reason,
        ] {
            assert_eq!(value.as_deref(), Some("visible"));
        }
    }

    #[test]
    fn debug_contains_data_without_connection_internals() {
        let client = crate::Client::disconnected_for_test();
        assert_eq!(format!("{client:?}"), "Client { .. }");

        let rendered = format!("{:?}", chat("PRIVMSG", "hello from Ergo"));
        assert!(rendered.contains("raw_text: \"hello from Ergo\""));
        for field in ["commands:", "connected:", "isupport:", "caps:"] {
            assert!(!rendered.contains(field), "{rendered}");
        }
    }

    #[test]
    fn routes_channels_and_private_messages_with_ergo_chantypes() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CHANTYPES=#"]);
        let channel = event(
            Line::privmsg("#papo", "hello").with_source("emily!~emily@localhost"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        assert!(!matches!(channel.conversation, Conversation::Private(_)));
        assert!(matches!(&channel.conversation, Conversation::Channel(name) if name == "#papo"));
        assert_eq!(channel.return_address(), "#papo");
        assert_eq!(channel.actor.nick(), "emily");

        let private = event(
            Line::privmsg("papo", "hello").with_source("emily!~emily@localhost"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        assert!(matches!(private.conversation, Conversation::Private(_)));
        assert!(matches!(&private.conversation, Conversation::Private(nick) if nick == "emily"));
        assert_eq!(private.return_address(), "emily");

        let server = parse_event(
            Line::notice("papo", "Server maintenance begins soon").with_source("irc.example"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        assert!(matches!(server, Event::ServerNotice(notice)
            if notice.kind == ServerNoticeKind::Notice
                && notice.to == Audience::Me));

        let broadcast = parse_event(
            Line::notice("$*.example.org", "Maintenance begins soon").with_source("oper!o@staff"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        assert!(matches!(broadcast, Event::ServerNotice(notice)
            if matches!(&notice.to, Audience::Broadcast(Broadcast::Mask(mask)) if mask == "$*.example.org")));
    }

    #[test]
    fn return_address_falls_back_to_the_classified_conversation() {
        let mapping = crate::Casemapping::default();
        let incomplete = Line::new("PRIVMSG");
        let channel = Conversation::Channel(channel(mapping.channel("#papo")));
        assert_eq!(
            super::return_address(&incomplete.as_ref(), &channel),
            "#papo"
        );

        let private_line = Line::privmsg("papo", "hello from Alice");
        let private = Conversation::Private(user_handle(mapping.nick("alice")));
        assert_eq!(
            super::return_address(&private_line.as_ref(), &private),
            "alice"
        );
    }

    #[test]
    fn raw_message_construction_does_not_trust_channel_context() {
        let tagged = |line| {
            event(
                LineRef::parse(line).unwrap().to_owned(),
                &Isupport::default(),
                "papo",
                UNIX_EPOCH,
            )
        };
        let channel = tagged(
            "@+channel-context=#other :emily!~emily@localhost PRIVMSG #papo :hello from Ergo",
        );
        assert_eq!(channel.channel_context, None);
        assert_eq!(channel.tags.get("+channel-context"), Some("#other"));

        let private =
            tagged("@+channel-context=#papo :emily!~emily@localhost PRIVMSG papo :hello from Ergo");
        assert_eq!(private.channel_context, None);
        assert_eq!(private.tags.get("+channel-context"), Some("#papo"));
    }

    #[test]
    fn separates_status_prefixes_from_the_channel_target() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CHANTYPES=#", "STATUSMSG=@+"]);
        let message = event(
            Line::privmsg("@+#ops", "operators and voiced users")
                .with_source("emily!~emily@localhost")
                .with_tag("bot", ""),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );

        assert_eq!(message.status.map(crate::Prefix::symbol), Some('+'));
        assert!(matches!(&message.conversation, Conversation::Channel(name) if name == "#ops"));
        assert_eq!(message.return_address(), "@+#ops");
        assert!(!matches!(message.conversation, Conversation::Private(_)));
        assert_eq!(message.tags.get("bot"), Some(""));
    }

    #[test]
    fn exposes_message_tags_and_server_time() {
        let received = UNIX_EPOCH + Duration::from_secs(42);
        let tagged = event(
            Line::privmsg("#papo", "hello")
                .with_source("emily!~emily@localhost")
                .with_tag("time", "2026-02-03T04:05:06.789Z")
                .with_tag("msgid", "RfcBTzyxY120004")
                .with_tag("account", "emily")
                .with_tag("bot", "")
                .with_tag("draft/relaymsg", "bridge-bot")
                .with_tag("+draft/reply", "RfcBTzyxY120003"),
            &Isupport::default(),
            "papo",
            received,
        );
        assert_eq!(
            tagged.server_time,
            Some(crate::ServerTime::from_stored_millis(1_770_091_506_789).unwrap())
        );
        assert!(matches!(&tagged.conversation, Conversation::Channel(name) if name == "#papo"));
        assert_eq!(tagged.msgid.as_deref(), Some("RfcBTzyxY120004"));
        assert!(matches!(tagged.actor.account(), Account::LoggedIn(account) if account == "emily"));
        let tagged_event = Event::Message(tagged.clone());
        assert_eq!(
            tagged_event.msgid().map(AsRef::as_ref),
            Some("RfcBTzyxY120004")
        );
        assert_eq!(tagged_event.server_time(), tagged.server_time);
        assert_eq!(tagged_event.received_at(), Some(received));
        assert_eq!(tagged.received_at, received);
        assert_eq!(
            super::unparsed_from_line(
                Line::new("VENDOR")
                    .param("payload")
                    .with_tag("msgid", "Other120004")
                    .with_tag("time", "2026-02-03T04:05:06.789Z"),
                received,
            )
            .msgid()
            .map(AsRef::as_ref),
            Some("Other120004")
        );
        assert_eq!(
            Event::Message(chat("PRIVMSG", "human message")).server_time(),
            None
        );
        let reply: Option<&MessageId> = tagged.in_reply_to.as_ref();
        assert_eq!(reply.unwrap(), "RfcBTzyxY120003");
        #[cfg(feature = "unstable")]
        assert_eq!(
            tagged.relayed_by.as_ref().map(papo_proto::Nick::as_str),
            Some("bridge-bot")
        );

        for time in [None, Some("not-a-time")] {
            let mut message =
                Line::privmsg("#papo", "fallback").with_source("emily!~emily@localhost");
            if let Some(time) = time {
                message = message.with_tag("time", time);
            }
            let message = event(message, &Isupport::default(), "papo", received);
            assert_eq!(message.server_time, None);
            assert_eq!(message.received_at, received);
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn every_person_event_carries_handle() {
        let received_at = UNIX_EPOCH + Duration::from_secs(42);
        let server_time = crate::ServerTime::from_stored_millis(1_770_091_506_789).unwrap();
        let convert = |line: &str| {
            let mut session = Session::new(Config::default().nick("papo"));
            session.handle_line(Instant::now(), line.as_bytes());
            let core = session.poll_event().expect("expected event from live line");
            match from_core_with_mapping(
                core,
                session.isupport().casemapping(),
                crate::Origin::NONE,
                session.caps().contains_key(&crate::cap::ACCOUNT_TAG),
                false,
                received_at,
            ) {
                CoreEvent::Forward(event) => event,
                CoreEvent::Line(super::CoreLine::Complete(line)) => super::incoming_from_core(
                    line,
                    crate::Origin::NONE,
                    session.isupport(),
                    session.caps().contains_key(&crate::cap::ACCOUNT_TAG),
                    false,
                    received_at,
                    None,
                ),
                _ => panic!("live line did not produce an event"),
            }
        };
        for (expected_variant, msgid, line) in MESSAGE_EVENT_METADATA_CASES {
            let event = convert(line);
            let actual_variant = match &event {
                Event::Message(_) => "Message",
                Event::Notice(_) => "Notice",
                Event::ServerNotice(_) => "ServerNotice",
                Event::CtcpQuery(_) => "CtcpQuery",
                Event::CtcpReply(_) => "CtcpReply",
                Event::BroadcastCtcp(_) => "BroadcastCtcp",
                Event::BroadcastTagMsg(_) => "BroadcastTagMsg",
                Event::TagMsg(_) => "TagMsg",
                Event::Reacted(_) => "Reacted",
                Event::Redacted(_) => "Redacted",
                Event::Joined(_) => "Joined",
                Event::Parted(_) => "Parted",
                Event::Kicked(_) => "Kicked",
                Event::Invited(_) => "Invited",
                Event::Quit(_) => "Quit",
                Event::NickChanged(_) => "NickChanged",
                Event::ChannelRenamed(_) => "ChannelRenamed",
                Event::ModeChanged(_) => "ModeChanged",
                Event::TopicChanged(_) => "TopicChanged",
                Event::Other(_) => "Other",
                _ => "unexpected",
            };
            assert_eq!(actual_variant, expected_variant);
            match expected_variant {
                "BroadcastCtcp" => {
                    let broadcast = event.as_broadcast_ctcp().unwrap();
                    assert_eq!(broadcast.command, "VERSION");
                    assert!(
                        matches!(&broadcast.to, Broadcast::Mask(mask) if mask == "$*.example.org")
                    );
                }
                "BroadcastTagMsg" => {
                    let broadcast = event.as_broadcast_tag_msg().unwrap();
                    assert!(matches!(broadcast.to, Broadcast::Everyone));
                    assert_eq!(broadcast.msgid().map(AsRef::as_ref), Some(msgid));
                    assert_eq!(broadcast.received_at, received_at);
                    assert_eq!(broadcast.tags.get("+typing"), Some("active"));
                    assert_eq!(broadcast.tags.get("+draft/react"), Some("👍"));
                }
                _ => {}
            }
            match &event {
                Event::Message(event) => assert!(!event.actor.nick().as_str().is_empty()),
                Event::Notice(event) => assert!(!event.actor.nick().as_str().is_empty()),
                Event::CtcpQuery(event) => assert!(!event.actor.nick().as_str().is_empty()),
                Event::CtcpReply(event) => assert!(!event.actor.nick().as_str().is_empty()),
                Event::TagMsg(event) => assert!(!event.actor.nick().as_str().is_empty()),
                Event::Reacted(event) => assert!(!event.actor.nick().as_str().is_empty()),
                Event::Joined(event) => assert!(!event.actor.nick().as_str().is_empty()),
                Event::Parted(event) => assert!(!event.actor.nick().as_str().is_empty()),
                Event::Kicked(event) => {
                    assert!(!event.subject.nick().as_str().is_empty());
                    assert!(event.actor.user().is_some());
                }
                Event::Invited(event) => {
                    assert!(!event.subject.nick().as_str().is_empty());
                    assert!(event.actor.user().is_some());
                }
                Event::Quit(event) => assert!(!event.actor.nick().as_str().is_empty()),
                Event::NickChanged(event) => assert!(!event.actor.nick().as_str().is_empty()),
                Event::ChannelRenamed(event) => assert!(event.actor.user().is_some()),
                Event::ModeChanged(event) => assert!(event.actor.user().is_some()),
                Event::TopicChanged(event) => assert!(event.actor.user().is_some()),
                Event::Redacted(event) => {
                    assert!(event.actor.user().is_some());
                }
                Event::ServerNotice(event) => {
                    if let Some(user) = event.actor.user() {
                        assert!(!user.nick().as_str().is_empty());
                    }
                }
                Event::BroadcastCtcp(event) => {
                    assert!(!event.actor.nick().as_str().is_empty());
                }
                Event::BroadcastTagMsg(event) => {
                    assert!(!event.actor.nick().as_str().is_empty());
                }
                Event::Other(_) => {}
                _ => panic!("unexpected event without a person-handle assertion"),
            }
            assert_eq!(
                event.msgid().map(AsRef::as_ref),
                Some(msgid),
                "{expected_variant}"
            );
            assert_event_times(&event, expected_variant, server_time, received_at);
            let account = match &event {
                Event::Parted(event) => Some(event.actor.account().clone()),
                Event::Kicked(event) => event.actor.user().map(|actor| actor.account().clone()),
                Event::Quit(event) => Some(event.actor.account().clone()),
                Event::NickChanged(event) => Some(event.actor.account().clone()),
                Event::ModeChanged(event) => {
                    event.actor.user().map(|actor| actor.account().clone())
                }
                Event::TopicChanged(event) => {
                    event.actor.user().map(|actor| actor.account().clone())
                }
                _ => None,
            };
            if let Some(account) = account {
                assert_eq!(account, Account::Unknown, "{expected_variant}");
            }
            match &event {
                Event::Reacted(event) => assert_eq!(event.actor.account(), &Account::Unknown),
                Event::Redacted(event) => {
                    assert_eq!(event.actor.user().unwrap().account(), &Account::Unknown);
                }
                _ => {}
            }
        }

        let untagged = convert(":emily!~emily@cloak.example PRIVMSG #papo :hello from Ergo");
        assert_eq!(untagged.msgid(), None);
        assert_eq!(untagged.server_time(), None);
    }

    #[test]
    fn all_channel_events_return_handles() {
        let convert = |line: &str| {
            let mut session = Session::new(Config::default().nick("papo"));
            session.handle_line(Instant::now(), line.as_bytes());
            let core = session.poll_event().expect("expected event from live line");
            match from_core_with_mapping(
                core,
                session.isupport().casemapping(),
                crate::Origin::NONE,
                false,
                false,
                UNIX_EPOCH,
            ) {
                CoreEvent::Forward(event) => event,
                CoreEvent::Line(super::CoreLine::Complete(line)) => super::incoming_from_core(
                    line,
                    crate::Origin::NONE,
                    session.isupport(),
                    false,
                    false,
                    UNIX_EPOCH,
                    None,
                ),
                _ => panic!("live line did not produce an event"),
            }
        };
        let cases = [
            (
                ":emily!~emily@cloak.example PRIVMSG #papo :release ready",
                Some("#papo"),
            ),
            (":emily!~emily@cloak.example JOIN #papo", Some("#papo")),
            (
                ":irc.example NOTICE #papo :Channel will be renamed at 18:00",
                Some("#papo"),
            ),
            (
                ":alice!~alice@cloak.example RENAME #papo #papo-dev :Project renamed",
                Some("#papo-dev"),
            ),
            (
                ":ChanServ!service@services.example MODE #papo +o emily",
                Some("#papo"),
            ),
            (":irc.example MODE papo +i", None),
            (":emily!~emily@cloak.example NICK emily_away", None),
            (":emily!~emily@cloak.example QUIT :Client Quit", None),
        ];

        for (line, expected) in cases {
            let conversation = convert(line).conversation();
            match (conversation.as_ref(), expected) {
                (Some(Conversation::Channel(channel)), Some(expected)) => {
                    assert_eq!(channel.name(), expected, "{line}");
                }
                (None, None) => {}
                _ => panic!("unexpected conversation {conversation:?} for {line}"),
            }
        }
    }

    #[test]
    fn derived_event_times_follow_the_trigger_without_sharing_its_msgid() {
        let received_at = UNIX_EPOCH + Duration::from_secs(42);
        let server_time = crate::ServerTime::from_stored_millis(1_770_091_506_789).unwrap();
        let cases = [
            (
                "AccountChanged",
                "@msgid=RfcBTzyxY120201;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example ACCOUNT emily",
            ),
            (
                "AwayChanged",
                "@msgid=RfcBTzyxY120202;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example AWAY :Lunch",
            ),
            (
                "HostChanged",
                "@msgid=RfcBTzyxY120203;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example CHGHOST emily users/emily",
            ),
            (
                "RealnameChanged",
                "@msgid=RfcBTzyxY120205;time=2026-02-03T04:05:06.789Z :emily!~emily@cloak.example SETNAME :Emily Smith",
            ),
            (
                "TypingChanged",
                "@msgid=RfcBTzyxY120204;time=2026-02-03T04:05:06.789Z;+typing=active :emily!~emily@cloak.example TAGMSG #papo",
            ),
        ];

        for (expected_variant, line) in cases {
            let mut session = Session::new(Config::default().nick("papo"));
            for setup in [
                ":papo!~papo@client.example JOIN #papo",
                ":emily!~emily@cloak.example JOIN #papo",
            ] {
                session.handle_line(Instant::now(), setup.as_bytes());
                while session.poll_event().is_some() {}
            }
            session.handle_line(Instant::now(), line.as_bytes());
            let event = std::iter::from_fn(|| session.poll_event())
                .filter_map(|event| {
                    match from_core_with_mapping(
                        event,
                        Casemapping::default(),
                        crate::Origin::NONE,
                        false,
                        false,
                        received_at,
                    ) {
                        CoreEvent::Forward(event) => Some(event),
                        _ => None,
                    }
                })
                .find(|event| {
                    matches!(
                        (expected_variant, event),
                        ("AccountChanged", Event::AccountChanged(_))
                            | ("AwayChanged", Event::AwayChanged(_))
                            | ("HostChanged", Event::HostChanged(_))
                            | ("RealnameChanged", Event::RealnameChanged(_))
                            | ("TypingChanged", Event::TypingChanged(_))
                    )
                })
                .expect("trigger produced the expected derived event");
            assert_eq!(event.msgid(), None, "{expected_variant}");
            assert_event_times(&event, expected_variant, server_time, received_at);
            let actor = match &event {
                Event::AccountChanged(event) => &event.actor,
                Event::AwayChanged(event) => &event.actor,
                Event::HostChanged(event) => &event.actor,
                Event::RealnameChanged(event) => &event.actor,
                Event::TypingChanged(event) => &event.actor,
                _ => unreachable!("the derived-event table is exhaustive"),
            };
            assert_eq!(actor.nick(), "emily", "{expected_variant}");
        }
    }

    #[test]
    fn typing_timeout_uses_its_observation_time_without_a_server_time() {
        let now = Instant::now();
        let received_at = UNIX_EPOCH + Duration::from_secs(73);
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            now,
            b"@+typing=active :emily!~emily@cloak.example TAGMSG #papo",
        );
        while session.poll_event().is_some() {}
        session.handle_timeout(papo_core::deadline(now, Duration::from_secs(6)).unwrap());
        let CoreEvent::Forward(event) = from_core_with_mapping(
            session.poll_event().expect("typing timeout event"),
            Casemapping::default(),
            crate::Origin::NONE,
            false,
            false,
            received_at,
        ) else {
            panic!("expected forwarded typing timeout")
        };
        assert!(matches!(event, Event::TypingChanged(_)));
        assert_eq!(event.msgid(), None);
        assert_eq!(event.server_time(), None);
        assert_eq!(event.received_at(), Some(received_at));
    }

    #[test]
    fn message_names_capture_the_active_casemapping() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CASEMAPPING=ascii", "NICKLEN=30"]);
        let message = event(
            Line::privmsg("papo", "hello")
                .with_source("Alice[bot]!alice@cloak.example")
                .with_tag("account", "Alice[bot]")
                .with_tag("draft/relaymsg", "Bridge[bot]"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );

        let nick = message.actor.nick();
        assert_eq!(nick.casemapping(), Casemapping::Ascii);
        assert!(matches!(
            message.actor.account(),
            Account::LoggedIn(account) if account.casemapping() == Casemapping::Ascii
        ));
        assert_ne!(nick, "alice{bot}");
        #[cfg(feature = "unstable")]
        assert_eq!(
            message.relayed_by.as_ref().unwrap().casemapping(),
            Casemapping::Ascii
        );
    }

    #[test]
    fn classifies_own_and_other_messages() {
        let classify = |message| event(message, &Isupport::default(), "Papo[bot]", UNIX_EPOCH);
        let echo = classify(Line::privmsg("#papo", "sent").with_source("papo{bot}!p@localhost"));
        assert!(echo.is_own());
        let tagged = classify(
            Line::privmsg("#papo", "earlier")
                .with_source("emily!e@localhost")
                .with_tag("batch", "history42"),
        );
        assert!(tagged.tags.contains("batch"));
        let live = classify(Line::privmsg("#papo", "now").with_source("emily!e@localhost"));
        assert!(!live.is_own());

        let route = |line, actor_party| {
            let isupport = Isupport::default();
            message_event_from_line(
                line,
                None,
                actor_party,
                Conversation::Channel(channel(isupport.casemapping().channel("#papo"))),
                None,
                None,
                &isupport,
                false,
                false,
                UNIX_EPOCH,
                None,
            )
        };
        assert!(matches!(route(
                Line::privmsg("#papo", "from another device").with_source("papo{bot}!p@localhost"),
                Party::Me), Event::Message(message)
            if message.actor_party == Party::Me));
        assert!(matches!(
            route(
                Line::privmsg("#papo", "from emily").with_source("emily!e@localhost"),
                Party::Other
            ),
            Event::Message(_)
        ));
        assert!(matches!(route(
                Line::notice("#papo", "from another device").with_source("papo{bot}!p@localhost"),
                Party::Me), Event::Notice(notice)
            if notice.actor_party == Party::Me));
    }
    fn forwarded(event: papo_core::Event) -> Event {
        let CoreEvent::Forward(event) = from_core_with_mapping(
            event,
            Casemapping::default(),
            crate::Origin::NONE,
            true,
            false,
            UNIX_EPOCH,
        ) else {
            panic!("event was not forwarded")
        };
        event
    }

    #[test]
    fn maps_feature_changes() {
        let event = forwarded(papo_core::Event::__features_changed(
            vec![Feature::Reactions, Feature::Typing],
            vec![Feature::History],
        ));
        let changed = event.as_features_changed().unwrap();
        assert_eq!(changed.added, [Feature::Reactions, Feature::Typing]);
        assert_eq!(changed.removed, [Feature::History]);
    }

    #[test]
    fn maps_casemapping_changes() {
        let event = forwarded(papo_core::Event::__casemapping_changed(
            Casemapping::Ascii,
            Casemapping::Rfc1459Strict,
        ));
        let changed = event.as_casemapping_changed().unwrap();
        assert_eq!(changed.old, Casemapping::Ascii);
        assert_eq!(changed.new, Casemapping::Rfc1459Strict);
    }

    #[test]
    fn maps_invitation_source_details() {
        let event = forwarded(papo_core::Event::__invited(
            Casemapping::default().channel("#papo"),
            Casemapping::default().nick("papo"),
            "emily!~emily@localhost".to_owned(),
            papo_core::Party::Other,
            papo_core::Party::Me,
        ));
        let invited = event.as_invited().unwrap();
        assert_eq!(invited.channel, "#papo");
        assert_eq!(invited.subject, "papo");
        assert!(matches!(&invited.actor, Source::User(user) if user.nick() == "emily"));
        assert_eq!(invited.subject_party, Party::Me);
    }

    #[test]
    fn forwarded_names_keep_the_core_casemapping() {
        let CoreEvent::Forward(Event::Joined(joined)) = from_core_with_mapping(
            papo_core::Event::__joined(
                papo_core::LineEnvelope::default(),
                "Alice[bot]!~a@cloak.example".to_owned(),
                None,
                Casemapping::Ascii.channel("#Papo[dev]"),
                Casemapping::Ascii.nick("Alice[bot]"),
                papo_core::ReplyToken::__unknown(crate::Origin::NONE),
                papo_core::Party::Other,
                None,
            ),
            Casemapping::Rfc1459,
            crate::Origin::NONE,
            false,
            false,
            UNIX_EPOCH,
        ) else {
            panic!("join was not forwarded");
        };

        assert_eq!(joined.channel.name().casemapping(), Casemapping::Ascii);
        let actor = joined.actor.nick();
        assert_eq!(actor.casemapping(), Casemapping::Ascii);
        assert_eq!(joined.actor.nick().casemapping(), Casemapping::Ascii);
        assert_ne!(joined.channel, "#papo{dev}");
        assert_ne!(joined.actor.nick(), "alice{bot}");
    }

    #[test]
    fn forwarded_event_payloads_keep_the_client_origin() {
        let origin = crate::Origin::__new(73);
        let mut config = Config::default();
        config.origin = origin;
        let mut session = Session::new(config);
        session.handle_line(
            Instant::now(),
            b"@account=alice :alice!~a@cloak.example JOIN #papo alice :Alice",
        );
        let CoreEvent::Forward(Event::Joined(joined)) = from_core_with_mapping(
            session.poll_event().expect("join event"),
            Casemapping::default(),
            origin,
            true,
            false,
            UNIX_EPOCH,
        ) else {
            panic!("join was not forwarded");
        };

        assert_eq!(joined.channel.origin(), origin);
        assert_eq!(joined.actor.nick().origin(), origin);
        assert_eq!(joined.actor, "alice");
        session.handle_line(
            Instant::now(),
            b"@account=alice :alice!~a@cloak.example PART #papo :deploying",
        );
        let CoreEvent::Forward(Event::Parted(parted)) = from_core_with_mapping(
            session.poll_event().expect("part event"),
            Casemapping::default(),
            origin,
            true,
            false,
            UNIX_EPOCH,
        ) else {
            panic!("part was not forwarded");
        };
        assert!(
            matches!(parted.actor.account(), Account::LoggedIn(account) if account.origin() == origin)
        );
    }

    #[test]
    fn queued_event_keeps_its_party_across_a_nick_change() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(now, b":papo!papo@localhost JOIN #papo");
        session.handle_line(now, b":papo!papo@localhost NICK papo-away");

        let CoreEvent::Forward(Event::Joined(joined)) = from_core_with_mapping(
            session.poll_event().unwrap(),
            Casemapping::Rfc1459,
            crate::Origin::NONE,
            false,
            false,
            UNIX_EPOCH,
        ) else {
            panic!("expected queued join");
        };
        let CoreEvent::Forward(Event::NickChanged(changed)) = from_core_with_mapping(
            session.poll_event().unwrap(),
            Casemapping::Rfc1459,
            crate::Origin::NONE,
            false,
            false,
            UNIX_EPOCH,
        ) else {
            panic!("expected queued nick change");
        };
        assert_eq!(joined.actor_party, Party::Me);
        assert_eq!(changed.actor_party, Party::Me);
    }

    fn mapped_message_event(event: papo_core::Event) -> Event {
        let CoreEvent::Line(super::CoreLine::Complete(message)) = from_core(event) else {
            panic!("message was not mapped")
        };
        super::incoming_from_core(
            message,
            crate::Origin::NONE,
            &Isupport::default(),
            false,
            false,
            UNIX_EPOCH,
            None,
        )
    }

    fn batched_event(kind: &str, command: &str) -> Event {
        let now = Instant::now();
        let mut session = Session::new(Config::default().max_batch_lines(0));
        session.handle_line(now, format!("BATCH +b {kind} #papo").as_bytes());
        session.handle_line(now, format!("@batch=b :alice!~a@host {command}").as_bytes());
        let Some(core) = session.poll_event() else {
            panic!("expected batch fallback")
        };
        let (line, incomplete) = match from_core(core) {
            CoreEvent::Line(line) => line.into_parts(),
            _ => panic!("{kind} fallback was not a message line"),
        };
        let event = super::incoming_from_core(
            line,
            crate::Origin::NONE,
            &Isupport::default(),
            false,
            false,
            UNIX_EPOCH,
            None,
        );
        if incomplete {
            super::mark_incomplete(event).unwrap()
        } else {
            event
        }
    }

    fn labelled_session(now: Instant, config: Config) -> Session {
        let mut session = Session::new(config);
        session.handle_connected(now);
        session.handle_line(now, b"CAP * LS :labeled-response");
        session.handle_line(now, b"CAP * ACK :labeled-response");
        while session.poll_transmit().is_some() {}
        session.handle_line(now, b":ergo.test 001 papo :Welcome to ErgoTest");
        while session.poll_event().is_some() {}
        session
    }

    #[test]
    fn maps_response() {
        let now = Instant::now();
        let mut session = labelled_session(now, Config::default());
        let id = session
            .request(now, Line::new("MODE").with_params(["#papo"]))
            .unwrap();
        session.handle_line(now, b"@label=0 :ergo.test ACK");
        assert!(
            matches!(from_core(session.poll_event().unwrap()), CoreEvent::Response(response) if response.id == id)
        );
    }

    #[test]
    fn maps_multiline_as_a_normal_message() {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        session.handle_line(now, b"BATCH +m draft/multiline #papo");
        session.handle_line(now, b"@batch=m :alice PRIVMSG #papo :first");
        session.handle_line(now, b"@batch=m :alice PRIVMSG #papo :second");
        session.handle_line(now, b"BATCH -m");
        assert!(matches!(
            from_core(session.poll_event().unwrap()),
            CoreEvent::Line(super::CoreLine::Complete(message))
                if message.as_ref().param(1) == Some("first\nsecond")
        ));
    }

    #[test]
    fn maps_tag_messages_with_typed_accessors() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(now, b":papo!~p@client.example JOIN #ops");
        while session.poll_event().is_some() {}
        session.handle_line(
            now,
            "@+draft/react=👍;+reply=RKi9Cw;draft/oper=network-admin :alice!~a@cloak TAGMSG #ops"
                .as_bytes(),
        );
        let event = forwarded(session.poll_event().expect("tag message"));
        let Event::Reacted(reaction) = event else {
            panic!("expected reaction: {event:?}")
        };
        assert_eq!(reaction.actor.nick(), "alice");
        assert!(
            matches!(&reaction.conversation, Conversation::Channel(channel) if channel == "#ops")
        );
        assert_eq!(reaction.return_address(), "#ops");
        assert_eq!(reaction.reaction, "👍");
        assert_eq!(reaction.reacts_to, "RKi9Cw");
        assert_eq!(reaction.action, crate::event::ReactionAction::Added);
    }

    #[test]
    fn maps_server_tag_messages_to_other() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            now,
            "@+typing=active;+draft/react=👍;+draft/reply=RKi9Cw :irc.example TAGMSG #papo"
                .as_bytes(),
        );
        let Some(papo_core::Event::Line(line)) = session.poll_event() else {
            panic!("expected untyped server TAGMSG");
        };
        let event = super::incoming_from_core(
            line,
            crate::Origin::NONE,
            &Isupport::default(),
            false,
            false,
            UNIX_EPOCH,
            None,
        );
        assert!(matches!(event, Event::Other(line) if line.as_ref().command_is("TAGMSG")));
    }

    #[test]
    fn distinguishes_every_typing_tag_value() {
        let typing = |value: &str| {
            let tag = papo_core::TagMessage {
                envelope: papo_core::LineEnvelope::default(),
                actor: "alice!~a@cloak".into(),
                actor_token: papo_core::ReplyToken::__unknown(crate::Origin::NONE),
                peer_token: papo_core::ReplyToken::__unknown(crate::Origin::NONE),
                actor_party: papo_core::Party::Other,
                conversation: papo_core::Conversation::Channel(
                    Casemapping::default().channel("#ops"),
                ),
                status: Some(crate::Prefix::__new('o', '@')),
                return_address: "@#ops".into(),
                tags: vec![("+typing".into(), Some(value.into()))],
            };
            match forwarded(papo_core::Event::TagMessage(tag)) {
                Event::TypingChanged(typing) => {
                    assert_eq!(typing.return_address(), "@#ops");
                    Some(typing.state)
                }
                Event::TagMsg(tag) => {
                    assert_eq!(tag.return_address(), "@#ops");
                    None
                }
                _ => None,
            }
        };
        assert_eq!(typing("active"), Some(Typing::Active));
        assert_eq!(typing("paused"), Some(Typing::Paused));
        assert_eq!(typing("done"), Some(Typing::Done));
        assert_eq!(typing("composing"), None);

        let tag = papo_core::TagMessage {
            envelope: papo_core::LineEnvelope::default(),
            actor: "alice!~a@cloak".into(),
            actor_token: papo_core::ReplyToken::__unknown(crate::Origin::NONE),
            peer_token: papo_core::ReplyToken::__unknown(crate::Origin::NONE),
            actor_party: papo_core::Party::Other,
            conversation: papo_core::Conversation::Channel(Casemapping::default().channel("#ops")),
            status: Some(crate::Prefix::__new('o', '@')),
            return_address: "@#ops".into(),
            tags: vec![("+typing".into(), Some("active".into()))],
        };
        assert!(
            matches!(forwarded(papo_core::Event::TagMessage(tag)), Event::TypingChanged(typing) if typing.state == Typing::Active)
        );

        let tag = papo_core::TagMessage {
            envelope: papo_core::LineEnvelope::default(),
            actor: "papo!~p@cloak".into(),
            actor_token: papo_core::ReplyToken::__unknown(crate::Origin::NONE),
            peer_token: papo_core::ReplyToken::__unknown(crate::Origin::NONE),
            actor_party: papo_core::Party::Me,
            conversation: papo_core::Conversation::Channel(Casemapping::default().channel("#ops")),
            status: None,
            return_address: "#ops".into(),
            tags: vec![("+typing".into(), Some("active".into()))],
        };
        assert!(matches!(forwarded(papo_core::Event::TagMessage(tag)),
            Event::TypingChanged(typing) if typing.actor_party == Party::Me));
    }

    #[test]
    fn forwards_core_typing_transitions() {
        let now = Instant::now();
        let mut session = papo_core::Session::new(papo_core::Config::default());
        session.handle_line(now, b"@+typing=active :alice!~a@cloak TAGMSG #papo");
        let event = session.poll_event().unwrap();
        assert!(matches!(
            forwarded(event),
            Event::TypingChanged(change)
                if change.state == Typing::Active
                    && change.actor.nick() == "alice"
        ));
    }

    #[test]
    fn maps_redactions() {
        let redacted = papo_core::Redacted {
            envelope: papo_core::LineEnvelope::default(),
            actor: "alice!~a@cloak".into(),
            actor_token: papo_core::ReplyToken::__unknown(crate::Origin::NONE),
            peer_token: papo_core::ReplyToken::__unknown(crate::Origin::NONE),
            actor_party: papo_core::Party::Other,
            conversation: papo_core::Conversation::Channel(Casemapping::default().channel("#ops")),
            status: Some(crate::Prefix::__new('o', '@')),
            return_address: "@#ops".into(),
            msgid: "RKi9Cw".into(),
            reason: Some(String::new()),
        };
        assert!(matches!(
            forwarded(papo_core::Event::Redacted(redacted.clone())),
            Event::Redacted(mapped)
                if matches!(mapped.actor, Source::User(ref user) if user.nick() == "alice")
                    && mapped.conversation == map_conversation(
                        redacted.conversation.clone(),
                        crate::Origin::NONE,
                    )
                    && mapped.status == redacted.status
                    && mapped.return_address() == "@#ops"
                    && mapped.redacts.as_ref() == redacted.msgid
                    && mapped.reason.is_none()
        ));
    }

    #[test]
    fn maps_request_timed_out() {
        let now = Instant::now();
        let mut session = labelled_session(now, Config::default().request_timeout(Duration::ZERO));
        let id = session
            .request(now, Line::new("MODE").with_params(["#papo"]))
            .unwrap();
        _ = session.poll_transmit();
        session.handle_timeout(now);
        assert!(
            matches!(from_core(session.poll_event().unwrap()), CoreEvent::RequestTimedOut(mapped) if mapped == id)
        );
    }

    #[test]
    fn maps_request_aborted() {
        let now = Instant::now();
        let mut session = labelled_session(now, Config::default());
        let id = session
            .request(now, Line::new("MODE").with_params(["#papo"]))
            .unwrap();
        session.handle_disconnected(now);
        assert!(
            matches!(from_core(session.poll_event().unwrap()), CoreEvent::RequestAborted { id: mapped, .. } if mapped == id)
        );
    }

    #[test]
    fn maps_transmit_aborted() {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        let token = session
            .send_tracked(now, Line::privmsg("#papo", "deployment queued"))
            .unwrap();
        session.handle_disconnected(now);
        assert!(
            matches!(from_core(session.poll_event().unwrap()), CoreEvent::TransmitAborted(mapped) if mapped == token)
        );
    }

    fn recipient_gone_event(request: bool) -> papo_core::Event {
        let now = Instant::now();
        let mut session = Session::new(
            Config::default()
                .nick("papo")
                .wanted_caps(["labeled-response"]),
        );
        session.handle_connected(now);
        session.handle_line(now, b"CAP * LS :labeled-response");
        session.handle_line(now, b"CAP * ACK :labeled-response");
        while session.poll_transmit().is_some() {}
        session.handle_line(now, b":ergo.test 001 papo :Welcome to ErgoTest");
        while session.poll_event().is_some() {}
        for line in [
            ":papo!p@localhost JOIN #review",
            ":alice!~alice@cloak.example JOIN #review",
            ":mallory!~mallory@users.example JOIN #review",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        while session.poll_event().is_some() {}
        let guard = session
            .state()
            .__capture_recipient(
                session.state().casemapping().nick("alice"),
                Account::Unknown,
            )
            .unwrap();
        if request {
            session
                .request_guarded_silent(
                    now,
                    Line::new("WHO").with_params(["alice", "%na"]),
                    Some(guard),
                )
                .unwrap();
        } else {
            session
                .send_guarded_tracked(
                    now,
                    Line::notice("alice", "classified release details"),
                    Some(guard),
                )
                .unwrap();
        }
        session.handle_line(now, b":alice!~alice@cloak.example QUIT :Client Quit");
        session.handle_line(now, b":mallory!~mallory@users.example NICK alice");
        while session.poll_event().is_some() {}
        assert!(session.poll_transmit().is_none());
        session.poll_event().expect("guard refusal event")
    }

    #[test]
    fn maps_every_recipient_gone_variant() {
        for (expected, event) in [
            ("RequestRecipientGone", recipient_gone_event(true)),
            ("TransmitRecipientGone", recipient_gone_event(false)),
        ] {
            let mapped = from_core(event);
            assert!(
                matches!(
                    (expected, mapped),
                    ("RequestRecipientGone", CoreEvent::RequestRecipientGone(_))
                        | ("TransmitRecipientGone", CoreEvent::TransmitRecipientGone(_))
                ),
                "{expected}"
            );
        }
    }

    #[test]
    fn maps_standard_reply() {
        for (core, public) in [
            (papo_proto::StandardReplyKind::Fail, StandardReplyKind::Fail),
            (papo_proto::StandardReplyKind::Warn, StandardReplyKind::Warn),
            (papo_proto::StandardReplyKind::Note, StandardReplyKind::Note),
        ] {
            let reply = CoreStandardReply {
                kind: core,
                command: "CHATHISTORY".into(),
                code: "RATE_LIMITED".into(),
                context: vec!["#papo".into()],
                description: "Please wait".into(),
                #[cfg(feature = "unstable")]
                refusal: papo_core::Refusal::RateLimited,
            };
            let Event::StandardReply(mapped) =
                forwarded(papo_core::Event::StandardReply(reply.clone()))
            else {
                panic!("standard reply was not forwarded")
            };
            assert_eq!(mapped.kind, public);
            assert_eq!(mapped.command.as_str(), reply.command);
            assert_eq!(mapped.code.as_str(), reply.code);
            assert_eq!(mapped.context, reply.context);
            assert_eq!(mapped.description, reply.description);
        }
    }

    #[test]
    fn maps_account_creation_results() {
        assert!(matches!(
            forwarded(papo_core::Event::__account_created(Casemapping::default().account("alice"), true)),
            Event::AccountCreation(crate::AccountOutcome::Created { account })
                if account == "alice"
        ));
        assert!(matches!(
            forwarded(papo_core::Event::__verification_required(Casemapping::default().account("alice"), "Verification code sent".into())),
            Event::AccountCreation(crate::AccountOutcome::VerificationRequired {
                account,
                message,
            })
                if account == "alice" && message == "Verification code sent"
        ));
    }

    #[test]
    fn maps_channel_renames() {
        assert!(matches!(
            forwarded(papo_core::Event::__channel_renamed(Casemapping::default().channel("#rust"), Casemapping::default().channel("#rust-lang"), "alice!a@localhost".into(), papo_core::Party::Other, Some("Project renamed".into()))),
            Event::ChannelRenamed(renamed)
                if renamed.old == "#rust"
                    && renamed.new == "#rust-lang"
                    && renamed.actor.nick().is_some_and(|nick| nick == "alice")
                    && renamed.reason.as_deref() == Some("Project renamed")
        ));

        assert!(matches!(
            forwarded(papo_core::Event::__channel_renamed(Casemapping::default().channel("#rust"), Casemapping::default().channel("#rust-lang"), "ergo.test".into(), papo_core::Party::Other, None)),
            Event::ChannelRenamed(renamed)
                if matches!(renamed.actor, Source::Server(ref name) if name == "ergo.test")
                    && renamed.reason.is_none()
        ));
    }

    #[test]
    fn maps_metadata_events() {
        assert!(matches!(
            forwarded(papo_core::Event::__metadata_changed(papo_core::Subject::Channel(Casemapping::default().channel("#papo")), papo_core::ReplyToken::__unknown(crate::Origin::NONE), "pronouns".into(), Some("they/them".into()), Visibility::Restricted("members".into()))),
            Event::MetadataChanged(changed)
                if matches!(changed.subject, crate::Subject::Channel(ref channel) if channel == "#papo")
                    && changed.key == "pronouns"
                    && matches!(changed.value, Some(ref value)
                        if value.value == "they/them"
                            && matches!(value.visibility, Visibility::Restricted(ref policy) if policy == "members"))
        ));
        assert!(matches!(
            forwarded(papo_core::Event::__metadata_changed(
                papo_core::Subject::User(Casemapping::default().nick("alice")),
                papo_core::ReplyToken::__unknown(crate::Origin::NONE),
                "avatar".into(),
                None,
                Visibility::Public
            )),
            Event::MetadataChanged(MetadataChanged { value: None, .. })
        ));
        assert!(matches!(
            from_core(papo_core::Event::__metadata_sync_later(
                "alice".into(),
                Some(Duration::from_secs(90))
            )),
            CoreEvent::Unknown
        ));
    }

    #[test]
    fn unsupported_batch_lines_stay_untyped() {
        for kind in ["znc.in/playback", "vendor/sync"] {
            assert!(matches!(
                batched_event(kind, "PRIVMSG #papo :earlier"),
                Event::Other(line) if line.as_ref().command_is("PRIVMSG")
            ));
        }

        let Event::Incomplete(Incomplete::Notice(partial)) =
            batched_event("draft/multiline", "NOTICE #papo :partial batch")
        else {
            panic!("expected incomplete multiline notice")
        };
        assert_eq!(partial.text, "partial batch");

        for kind in ["netsplit", "netjoin"] {
            let Event::Notice(live) = batched_event(kind, "NOTICE papo :live batch") else {
                panic!("expected live fallback notice")
            };
            assert_eq!(live.text, "live batch");
        }
    }

    #[test]
    fn historical_private_messages_use_the_batch_or_actor_conversation() {
        let client = crate::Client::disconnected_for_test();
        let batch = Conversation::Private(user_handle(Casemapping::default().nick("alice")));
        for history_conversation in [Some(&batch), None] {
            let Event::Message(message) = historical_message_event(
                Line::privmsg("papo_away", "can you send me the invoice?")
                    .with_source("alice!~a@host"),
                Party::Other,
                history_conversation,
                &client,
                UNIX_EPOCH,
            ) else {
                panic!("expected a historical message")
            };
            assert!(matches!(
                message.conversation,
                Conversation::Private(ref nick) if nick == "alice"
            ));
        }
    }

    #[test]
    fn historical_channel_messages_retain_the_joined_channel_handle() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            now,
            b":ergo.test 005 papo CASEMAPPING=rfc1459 CHANTYPES=# :are supported",
        );
        session.handle_line(now, b":papo!p@localhost JOIN #review * :Papo Driver");
        let (client, _) = crate::driver::test_support::client_with_commands(&session);
        let channel = client.channel("#review").unwrap();

        let Event::Message(message) = historical_message_event(
            Line::privmsg("#review", "release approved").with_source("alice!~a@staff.example"),
            Party::Other,
            None,
            &client,
            UNIX_EPOCH,
        ) else {
            panic!("expected a historical message")
        };
        assert_eq!(message.conversation, Conversation::Channel(channel));
    }

    #[test]
    fn live_peer_pin_prefers_the_received_account_over_stale_state() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_connected_with(
            now,
            Connection {
                host: "irc.example".into(),
                security: ConnectionSecurity::Secure,
                authenticated_server_name: Some("irc.example".into()),
            },
        );
        for line in [
            ":irc.example 005 papo CASEMAPPING=rfc1459 :are supported",
            ":papo!~papo@localhost JOIN #review * :Papo",
            ":alice!~alice@cloak.example JOIN #review stale-account :Alice",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        let state = crate::State::from_session(&session, true);
        let conversation =
            Conversation::Private(user_handle(state.name_rules().unwrap().nick("alice")));
        let mut event = message_event_from_line(
            Line::privmsg("papo", "release ready")
                .with_source("alice!~alice@cloak.example")
                .with_tag("account", "fresh-account"),
            None,
            Party::Other,
            conversation,
            None,
            None,
            state.isupport().inner(),
            true,
            false,
            UNIX_EPOCH,
            None,
        );

        let shared = std::sync::Arc::new(std::sync::RwLock::new(std::sync::Arc::new(state)));
        super::attach_message_handles(
            &mut event,
            super::LineTokens {
                actor: None,
                peer: None,
            },
            Some(&shared),
            false,
        );
        let actor = &event.as_message().unwrap().actor;
        assert_eq!(
            actor.key().as_verified_str(),
            Some(
                "papo-peer:1;server=irc.example;nick-casemap=rfc1459;nick=alice;proof=account;account-casemap=rfc1459;account=fresh-account"
            )
        );
    }

    fn live_message_handle_event(line: &str) -> (Event, Arc<std::sync::RwLock<Arc<crate::State>>>) {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        for setup in [
            ":ergo.test 005 papo CASEMAPPING=rfc1459 CHANTYPES=# :are supported",
            ":papo!papo@localhost JOIN #review",
            ":alice!alice@staff.example JOIN #review",
        ] {
            session.handle_line(now, setup.as_bytes());
        }
        while session.poll_event().is_some() {}
        session.handle_line(now, line.as_bytes());
        let core = session.poll_event().expect("message event");
        let state = Arc::new(std::sync::RwLock::new(Arc::new(
            crate::State::from_session(&session, true),
        )));
        let event = match super::from_core_with_receipt(
            core,
            session.isupport().casemapping(),
            crate::Origin::NONE,
            false,
            false,
            UNIX_EPOCH,
            Some(session.state()),
            Some(&state),
        ) {
            CoreEvent::Forward(event) => event,
            CoreEvent::Line(super::CoreLine::Complete(line)) => super::incoming_from_core(
                line,
                crate::Origin::NONE,
                session.isupport(),
                false,
                false,
                UNIX_EPOCH,
                Some(&state),
            ),
            _ => panic!("unexpected mapped event"),
        };
        (event, state)
    }

    fn message_handle_actor(
        event: &Event,
    ) -> (&'static str, &crate::UserHandle, Option<&Conversation>) {
        match event {
            Event::Message(event) => ("Message", &event.actor, Some(&event.conversation)),
            Event::Incomplete(Incomplete::Message(event)) => {
                ("IncompleteMessage", &event.actor, Some(&event.conversation))
            }
            Event::Notice(event) => ("Notice", &event.actor, Some(&event.conversation)),
            Event::Incomplete(Incomplete::Notice(event)) => {
                ("IncompleteNotice", &event.actor, Some(&event.conversation))
            }
            Event::CtcpQuery(event) => ("CtcpQuery", &event.actor, Some(&event.conversation)),
            Event::CtcpReply(event) => ("CtcpReply", &event.actor, Some(&event.conversation)),
            Event::TagMsg(event) => ("TagMsg", &event.actor, Some(&event.conversation)),
            Event::Reacted(event) => ("Reacted", &event.actor, Some(&event.conversation)),
            Event::TypingChanged(event) => {
                ("TypingChanged", &event.actor, Some(&event.conversation))
            }
            Event::Redacted(event) => (
                "Redacted",
                event.actor.user().unwrap(),
                Some(&event.conversation),
            ),
            Event::ServerNotice(event) => ("ServerNotice", event.actor.user().unwrap(), None),
            Event::BroadcastCtcp(event) => ("BroadcastCtcp", &event.actor, None),
            Event::BroadcastTagMsg(event) => ("BroadcastTagMsg", &event.actor, None),
            event => panic!("unexpected event: {event:?}"),
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn every_message_event_actor_carries_a_live_handle() {
        let lines = [
            (
                "Message",
                "@msgid=ErgoHandle1 :alice!alice@staff.example PRIVMSG papo :release approved",
            ),
            (
                "Notice",
                "@msgid=ErgoHandle2 :alice!alice@staff.example NOTICE papo :deployment complete",
            ),
            (
                "CtcpQuery",
                ":alice!alice@staff.example PRIVMSG papo :\u{1}VERSION\u{1}",
            ),
            (
                "CtcpReply",
                ":alice!alice@staff.example NOTICE papo :\u{1}VERSION papo 0.1\u{1}",
            ),
            (
                "TagMsg",
                "@msgid=ErgoHandle3;+example/status=ready :alice!alice@staff.example TAGMSG papo",
            ),
            (
                "Reacted",
                "@+draft/react=✅;+draft/reply=ErgoHandle1 :alice!alice@staff.example TAGMSG papo",
            ),
            (
                "TypingChanged",
                "@+typing=active :alice!alice@staff.example TAGMSG papo",
            ),
            (
                "Redacted",
                ":alice!alice@staff.example REDACT papo ErgoHandle1 :superseded",
            ),
            (
                "ServerNotice",
                ":alice!alice@staff.example WALLOPS :scheduled maintenance",
            ),
            (
                "BroadcastCtcp",
                ":alice!alice@staff.example PRIVMSG * :\u{1}VERSION\u{1}",
            ),
            (
                "BroadcastTagMsg",
                "@+example/status=ready :alice!alice@staff.example TAGMSG *",
            ),
        ];
        let mut cases = lines
            .into_iter()
            .map(|(name, line)| {
                let (mut event, state) = live_message_handle_event(line);
                if let Event::Redacted(redacted) = &mut event {
                    let actor = user_handle(redacted.actor.user().unwrap().nick());
                    redacted.actor = Source::User(actor);
                    redacted.conversation =
                        Conversation::Private(user_handle(Casemapping::default().nick("mallory")));
                }
                let token = state.read().unwrap().core.__reply_token_for("alice");
                super::attach_message_handles(
                    &mut event,
                    super::LineTokens {
                        actor: Some(token),
                        peer: None,
                    },
                    Some(&state),
                    false,
                );
                (name, event)
            })
            .collect::<Vec<_>>();

        for (name, line) in [
            (
                "IncompleteMessage",
                ":alice!alice@staff.example PRIVMSG papo :partial release note",
            ),
            (
                "IncompleteNotice",
                ":alice!alice@staff.example NOTICE papo :partial deployment notice",
            ),
        ] {
            let (event, state) = live_message_handle_event(line);
            let mut event = match event {
                Event::Message(mut event) => {
                    event.actor = user_handle(event.actor.nick());
                    event.conversation =
                        Conversation::Private(user_handle(Casemapping::default().nick("mallory")));
                    Event::Incomplete(Incomplete::Message(event))
                }
                Event::Notice(mut event) => {
                    event.actor = user_handle(event.actor.nick());
                    event.conversation =
                        Conversation::Private(user_handle(Casemapping::default().nick("mallory")));
                    Event::Incomplete(Incomplete::Notice(event))
                }
                _ => unreachable!(),
            };
            let token = state.read().unwrap().core.__reply_token_for("alice");
            super::attach_message_handles(
                &mut event,
                super::LineTokens {
                    actor: Some(token),
                    peer: None,
                },
                Some(&state),
                false,
            );
            cases.push((name, event));
        }

        for (expected, event) in cases {
            let (actual, actor, conversation) = message_handle_actor(&event);
            assert_eq!(actual, expected);
            assert!(actor.proof().is_some(), "{expected}");
            assert!(actor == "ALICE", "{expected}");
            assert!(actor != "mallory", "{expected}");
            assert_eq!(crate::Target::__parts(actor).user.as_ref(), Some(actor));
            if let Some(Conversation::Private(peer)) = conversation {
                assert_eq!(peer, actor, "{expected}");
            }
        }
    }

    #[test]
    fn classifies_an_own_message() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("Papo[bot]"));
        session.handle_line(now, b":papo{bot}!~p@host PRIVMSG #papo :sent");
        let event = mapped_message_event(session.poll_event().expect("expected echo"));
        let echo = event.as_message().expect("expected own message");
        assert!(echo.is_own());
    }

    #[test]
    fn maps_joined() {
        let event = forwarded(papo_core::Event::__joined(
            papo_core::LineEnvelope::default(),
            "alice!~a@host".to_owned(),
            Some("Alice Liddell".to_owned()),
            Casemapping::default().channel("#papo"),
            Casemapping::default().nick("alice"),
            papo_core::ReplyToken::__unknown(crate::Origin::NONE),
            papo_core::Party::Other,
            None,
        ));
        let joined = event.as_joined().unwrap();
        assert_eq!(joined.channel, "#papo");
        assert_eq!(joined.actor.nick(), "alice");
        assert_eq!(joined.netjoin, None);
    }

    fn departure_actor(
        event: &Event,
    ) -> Option<(&'static str, &crate::UserHandle, &papo_core::User)> {
        match event {
            Event::Parted(event) => Some(("Parted", &event.actor, event.facts.as_ref()?)),
            _ => None,
        }
    }

    #[test]
    fn departure_events_keep_facts_removed_from_state() {
        let cases = [(
            "Parted",
            "@account=alice;bot :alice!~a@staff.example PART #papo :Deploying release",
        )];

        for (expected, line) in cases {
            let now = Instant::now();
            let mut session = Session::new(Config::default().nick("papo"));
            session.handle_line(
                now,
                b":ergo.test 005 papo BOT=B CHANMODES=Ibe,k,fl,CEMRUimnstu PREFIX=(ov)@+ :are supported",
            );
            session.handle_line(now, b":papo!~p@client.example JOIN #papo");
            session.handle_line(
                now,
                b"@account=alice :alice!~a@staff.example JOIN #papo alice :Alice Example",
            );
            while session.poll_event().is_some() {}

            session.handle_line(now, line.as_bytes());
            let mapped = from_core_with_mapping(
                session.poll_event().expect("departure event"),
                session.isupport().casemapping(),
                crate::Origin::NONE,
                true,
                true,
                UNIX_EPOCH,
            );
            let CoreEvent::Forward(event) = mapped else {
                panic!("{expected} was not forwarded")
            };
            let Some((actual, actor, facts)) = departure_actor(&event) else {
                panic!("{expected} had no departure facts: {event:?}")
            };
            assert_eq!(actual, expected);
            assert_eq!(actor.nick(), "alice");
            assert_eq!(facts.username(), Some("~a"));
            assert_eq!(facts.host(), Some("staff.example"));
            assert!(matches!(facts.account(), Account::LoggedIn(account) if account == "alice"));
            assert_eq!(facts.realname(), Some("Alice Example"));
        }
    }

    #[test]
    fn live_activity_uses_receipt_time_without_server_time() {
        let now = Instant::now();
        let received_at = UNIX_EPOCH + Duration::from_secs(1_792_520_531);
        let mut session = Session::new(Config::default());
        session.handle_line(
            now,
            b"@account=emily;msgid=RfcBTzyxY120012 :emily!~e@cloak.example JOIN #papo",
        );
        let mapped = from_core_with_mapping(
            session.poll_event().expect("join event"),
            Casemapping::default(),
            crate::Origin::NONE,
            false,
            false,
            received_at,
        );
        let CoreEvent::Forward(Event::Joined(joined)) = mapped else {
            panic!("expected mapped join");
        };
        assert_eq!(joined.msgid.as_deref(), Some("RfcBTzyxY120012"));
        assert_eq!(joined.received_at, received_at);
        let joined_event = Event::Joined(joined.clone());
        assert_eq!(joined_event.server_time(), None);
        assert_eq!(joined_event.received_at(), Some(received_at));
        assert_eq!(joined.actor, "emily");
    }

    #[test]
    fn maps_channel_synced() {
        assert!(matches!(
            forwarded(papo_core::Event::__channel_synced(Casemapping::default().channel("#papo"))),
            Event::ChannelSynced(synced) if synced.channel == "#papo"
        ));
    }

    #[test]
    fn maps_members_refreshed() {
        assert!(matches!(
            forwarded(papo_core::Event::__members_refreshed(Casemapping::default().channel("#papo"))),
            Event::MembersRefreshed(refreshed) if refreshed.channel == "#papo"
        ));
    }

    #[test]
    fn maps_parted() {
        assert!(matches!(
            forwarded(papo_core::Event::__parted(
                papo_core::LineEnvelope::default(),
                Casemapping::default().channel("#papo"),
                "emily!~emily@cloak.example".into(),
                Casemapping::default().nick("emily"),
                papo_core::ReplyToken::__unknown(papo_proto::Origin::NONE),
                None,
                Some("Leaving for lunch".into()),
                papo_core::Party::Other,
            )),
            Event::Parted(parted)
                if parted.channel == "#papo"
                    && parted.actor.nick() == "emily"
                    && parted.actor.account() == &Account::LoggedOut
                    && parted.reason.as_deref() == Some("Leaving for lunch")
        ));
    }

    #[test]
    fn maps_kicked() {
        assert!(matches!(
            forwarded(papo_core::Event::__kicked(
                papo_core::LineEnvelope::default(),
                Casemapping::default().channel("#papo"),
                Casemapping::default().nick("emily"),
                papo_core::ReplyToken::__unknown(papo_proto::Origin::NONE),
                None,
                papo_core::Source::from_proto(
                    papo_proto::Source::__user(
                        "alice",
                        Some("~alice"),
                        Some("staff.example"),
                    ),
                    Casemapping::default(),
                ),
                papo_core::ReplyToken::__unknown(papo_proto::Origin::NONE),
                papo_core::Party::Other,
                papo_core::Party::Other,
                Some("Flooding".into()),
            )),
            Event::Kicked(kicked)
                if kicked.channel == "#papo"
                    && kicked.subject == "emily"
                    && kicked.facts.is_none()
                    && kicked.actor.nick().is_some_and(|nick| nick == "alice")
                    && kicked.reason.as_deref() == Some("Flooding")
        ));
    }

    #[test]
    fn maps_quit() {
        assert!(matches!(
            forwarded(papo_core::Event::__quit(
                papo_core::LineEnvelope::default(),
                "emily!~emily@cloak.example".into(),
                Casemapping::default().nick("emily"),
                papo_core::ReplyToken::__unknown(papo_proto::Origin::NONE),
                None,
                Some("Client exited".into()),
                vec![
                    Casemapping::default().channel("#papo"),
                    Casemapping::default().channel("#rust"),
                ],
                papo_core::Party::Other,
                None,
            )),
            Event::Quit(quit)
                if quit.actor.nick() == "emily"
                    && quit.reason.as_deref() == Some("Client exited")
                    && quit.channels == ["#papo", "#rust"]
                    && quit.netsplit.is_none()
        ));
    }

    #[test]
    fn maps_split_membership_markers() {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        for line in [
            ":irc.example BATCH +join-42 netjoin irc.example irc.remote",
            "@batch=join-42 :alice!~a@remote.example JOIN #papo * :Alice Liddell",
            ":irc.example BATCH -join-42",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        let joined = forwarded(session.poll_event().expect("netjoin event"));
        assert!(matches!(
            joined.as_joined().and_then(|event| event.netjoin.as_ref()),
            Some(split) if split.batch == "join-42"
                && split.first == "irc.example"
                && split.second == "irc.remote"
        ));

        for line in [
            ":irc.example BATCH +split-42 netsplit irc.example irc.remote",
            "@batch=split-42 :alice!~a@remote.example QUIT :irc.example irc.remote",
            ":irc.example BATCH -split-42",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        let quit = forwarded(session.poll_event().expect("netsplit event"));
        assert!(matches!(
            quit.as_quit().and_then(|event| event.netsplit.as_ref()),
            Some(split) if split.batch == "split-42"
                && split.first == "irc.example"
                && split.second == "irc.remote"
        ));
    }

    #[test]
    fn maps_nick_changed() {
        let Event::NickChanged(changed) = forwarded(papo_core::Event::__nick_changed(
            papo_core::LineEnvelope::default(),
            "emily!~emily@cloak.example".into(),
            Casemapping::default().nick("emily"),
            Casemapping::default().nick("em"),
            papo_core::ReplyToken::__unknown(crate::Origin::NONE),
            vec![Casemapping::default().channel("#papo")],
            papo_core::Party::Other,
        )) else {
            panic!("expected nickname change")
        };
        assert_eq!(changed.actor.nick(), "em");
        assert_eq!(changed.old, "emily");
        assert_eq!(changed.new, "em");
        assert_eq!(changed.channels, ["#papo"]);
    }

    #[test]
    fn maps_account_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::__account_changed(
                papo_core::LineEnvelope::default(),
                "emily!~em@users/emily".into(),
                Casemapping::default().nick("emily"),
                Account::LoggedIn(Casemapping::default().account("emily")),
                papo_core::ReplyToken::__unknown(Casemapping::default().nick("emily").origin()),
                papo_core::Party::Other,
            )),
            Event::AccountChanged(changed)
                if changed.actor.nick() == "emily" && changed.new.name().is_some_and(|account| account == "emily")
        ));
    }

    #[test]
    fn maps_away_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::__away_changed(
                papo_core::LineEnvelope::default(),
                "emily!~em@users/emily".into(),
                Casemapping::default().nick("emily"),
                Away::__away_value(Some("Gone to lunch".into())),
                papo_core::ReplyToken::__unknown(Casemapping::default().nick("emily").origin()),
                papo_core::Party::Other,
            )),
            Event::AwayChanged(changed)
                if changed.actor.nick() == "emily"
                    && changed.new == Away::__away_value(Some("Gone to lunch".into()))
        ));
    }

    #[test]
    fn maps_away_numeric_actors_from_the_numeric_nick() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("emily"));
        session.handle_line(now, b":emily!em@127.0.0.1 JOIN #papo");
        _ = session.poll_event();
        session.handle_line(now, b":dan!~dan@cloak.example JOIN #papo");
        _ = session.poll_event();
        session.handle_line(now, b":ergo.test 301 emily dan :Gone to lunch");

        let away = forwarded(session.poll_event().expect("away event"));
        let away = away.as_away_changed().expect("typed away event");
        assert_eq!(away.actor.nick(), "dan");
        assert_eq!(away.actor_party, Party::Other);
    }

    #[test]
    fn maps_host_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::__host_changed(
                papo_core::LineEnvelope::default(),
                "emily!em@127.0.0.1".into(),
                Casemapping::default().nick("emily"),
                "~em".into(),
                "users/emily".into(),
                papo_core::ReplyToken::__unknown(Casemapping::default().nick("emily").origin()),
                papo_core::Party::Other,
            )),
            Event::HostChanged(changed)
                if changed.actor.nick() == "emily" && changed.new_username == "~em" && changed.new_host == "users/emily"
        ));
    }

    #[test]
    fn maps_realname_changed() {
        assert!(matches!(
            mapped_live_event(
                "@time=1970-01-01T00:00:42.000Z :emily!~em@users/emily SETNAME :Emily Smith",
                Instant::now(),
            ),
            Event::RealnameChanged(changed)
                if changed.actor.nick() == "emily"
                    && changed.new == "Emily Smith"
                    && changed.server_time == Some(crate::ServerTime::from_stored_millis(42_000).unwrap())
                    && changed.received_at == UNIX_EPOCH
        ));
    }

    #[test]
    fn maps_mode_changed() {
        let mode_changes = vec![papo_core::ModeChange::__new(
            'o',
            papo_core::ModeAction::Add,
            Some(papo_core::ModeArg::__member(
                Casemapping::default().nick("emily"),
                crate::Prefix::__new('o', '@'),
                papo_core::ReplyToken::__unknown(crate::Origin::NONE),
            )),
        )];
        assert!(matches!(
            forwarded(papo_core::Event::__mode_changed(papo_core::LineEnvelope::default(), papo_core::Subject::Channel(Casemapping::default().channel("#papo")), papo_core::ReplyToken::__unknown(papo_proto::Origin::NONE), papo_core::Source::from_proto(papo_proto::Source::__user("alice", Some("~alice"), Some("staff.example")), Casemapping::default()), papo_core::ReplyToken::__unknown(papo_proto::Origin::NONE), papo_core::Party::Other, papo_core::Party::Other, Some(Casemapping::default().nick("emily")), mode_changes.clone())),
            Event::ModeChanged(mode)
                if matches!(mode.subject, crate::Subject::Channel(ref channel) if channel == "#papo")
                    && mode.actor.nick().is_some_and(|nick| nick == "alice")
                    && mode.changes.len() == 1
                    && mode.changes[0].action == ModeAction::Add
                    && mode.changes[0].mode == 'o'
                    && matches!(&mode.changes[0].arg, Some(ModeArg::Member { user, .. }) if user == "emily")
        ));
    }

    #[test]
    fn nests_account_tags_on_server_capable_mode_actors() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("emily"));
        session.handle_line(
            now,
            b":irc.example 005 emily CHANMODES=Ibe,k,fl,CEMRUimnstu PREFIX=(ov)@+ :are supported",
        );
        while session.poll_event().is_some() {}
        session.handle_line(now, b":emily!em@127.0.0.1 JOIN #papo");
        assert!(matches!(
            session.poll_event(),
            Some(papo_core::Event::Joined { .. })
        ));
        session.handle_line(
            now,
            b"@account=services :ChanServ!service@services.example MODE #papo +o emily",
        );

        let mode = forwarded(session.poll_event().expect("mode event"));
        assert!(matches!(
            mode.as_mode_changed()
                .and_then(|event| event.actor.user())
                .map(|actor| actor.account().clone()),
            Some(Account::LoggedIn(account)) if account == "services"
        ));
    }

    #[test]
    fn mode_about_uses_the_build_time_nick() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("emily"));
        session.handle_line(
            now,
            b":irc.example 005 emily CHANMODES=Ibe,k,fl,CEMRUimnstu PREFIX=(ov)@+ :are supported",
        );
        while session.poll_event().is_some() {}
        session.handle_line(now, b":emily!em@127.0.0.1 JOIN #papo");
        assert!(matches!(
            session.poll_event(),
            Some(papo_core::Event::Joined { .. })
        ));
        session.handle_line(
            now,
            b":ChanServ!service@services.example MODE #papo +o emily",
        );

        let event = forwarded(session.poll_event().expect("mode event"));
        let mode = event.as_mode_changed().expect("typed mode event");
        assert!(mode.actor.nick().is_some_and(|nick| nick == "ChanServ"));
        assert_eq!(mode.actor_party, Party::Other);
        assert_eq!(
            mode.own_nick.as_ref().map(papo_proto::Nick::as_str),
            Some("emily")
        );
        assert!(
            matches!(&mode.changes[0].arg, Some(ModeArg::Member { user, .. }) if user == "emily"),
            "{:?}",
            mode.changes[0]
        );
        assert_eq!(mode.subject_party(&mode.changes[0]), Party::Me);
    }

    fn mode_event(subject: Subject, own_nick: Option<&str>) -> super::ModeChanged {
        let mapping = Casemapping::Rfc1459;
        super::ModeChanged {
            msgid: None,
            server_time: None,
            received_at: SystemTime::UNIX_EPOCH,
            subject,
            actor: crate::__testing::source(
                papo_proto::Source::__user("ChanServ", Some("service"), Some("services.example")),
                mapping,
            ),
            actor_party: Party::Other,
            changes: Vec::new(),
            own_nick: own_nick.map(|nick| mapping.nick(nick)),
        }
    }

    #[test]
    fn mode_about_covers_nick_arguments_user_subjects_and_history() {
        let mapping = Casemapping::Rfc1459;
        let own = ModeChange::new(
            'o',
            ModeAction::Add,
            Some(ModeArg::Member {
                user: user_handle(mapping.nick("emily")),
                prefix: crate::Prefix::__new('o', '@'),
            }),
        );
        let other = ModeChange::new(
            'v',
            ModeAction::Add,
            Some(ModeArg::Member {
                user: user_handle(mapping.nick("alice")),
                prefix: crate::Prefix::__new('v', '+'),
            }),
        );
        let channel_mode = mode_event(
            Subject::Channel(channel_handle(mapping.channel("#papo"))),
            Some("emily"),
        );
        assert_eq!(channel_mode.subject_party(&own), Party::Me);
        assert_eq!(channel_mode.subject_party(&other), Party::Other);
        assert_eq!(
            mode_event(
                Subject::Channel(channel_handle(mapping.channel("#papo"))),
                None,
            )
            .subject_party(&own),
            Party::Unknown
        );

        let user_mode = ModeChange::new('i', ModeAction::Add, None);
        for (subject, own_nick, expected) in [
            ("papo{bot}", Some("Papo[Bot]"), Party::Me),
            ("alice", Some("Papo[Bot]"), Party::Other),
            ("alice", None, Party::Unknown),
        ] {
            assert_eq!(
                mode_event(Subject::User(user_handle(mapping.nick(subject))), own_nick,)
                    .subject_party(&user_mode),
                expected
            );
        }
        assert_eq!(channel_mode.subject_party(&user_mode), Party::Other);
    }

    #[test]
    fn core_changes_keep_every_argument_kind() {
        let names = Casemapping::Rfc1459;
        let core = |mode, arg| papo_core::ModeChange::__new(mode, ModeAction::Add, arg);
        let cases = [
            core(
                'o',
                Some(papo_core::ModeArg::__member(
                    names.nick("Alice"),
                    crate::Prefix::__new('o', '@'),
                    papo_core::ReplyToken::__unknown(crate::Origin::NONE),
                )),
            ),
            core(
                'b',
                Some(papo_core::ModeArg::Mask(names.mask("*!*@spam.example"))),
            ),
            core(
                'k',
                Some(papo_core::ModeArg::Key(crate::Secret::new(
                    "review-key-2026",
                ))),
            ),
            core('l', Some(papo_core::ModeArg::Value("50".into()))),
            core('m', None),
        ];
        let mapped: Vec<_> = cases
            .into_iter()
            .map(|change| crate::ModeChange::from_core(change, |nick, _| user_handle(nick)))
            .collect();

        assert!(
            matches!(&mapped[0].arg, Some(ModeArg::Member { user, prefix }) if user == "alice" && prefix.symbol() == '@')
        );
        assert!(matches!(&mapped[1].arg, Some(ModeArg::Mask(mask)) if mask == "*!*@spam.example"));
        assert!(
            matches!(&mapped[2].arg, Some(ModeArg::Key(key)) if key.expose() == "review-key-2026")
        );
        assert!(matches!(&mapped[3].arg, Some(ModeArg::Value(value)) if value == "50"));
        assert_eq!(mapped[4].arg, None);
        assert_eq!(
            mapped.iter().map(|change| change.mode).collect::<String>(),
            "obklm"
        );
        assert!(mapped.iter().all(|change| change.action == ModeAction::Add));
    }

    #[test]
    fn typed_mode_arguments_keep_the_client_origin() {
        let origin = crate::Origin::__new(73);
        for (mode, kind, expected) in [
            (
                'x',
                papo_proto::ModeKind::Prefix(crate::Prefix::__new('x', '!')),
                "nick",
            ),
            ('x', papo_proto::ModeKind::List, "mask"),
            ('k', papo_proto::ModeKind::AlwaysArg, "key"),
            ('x', papo_proto::ModeKind::AlwaysArg, "value"),
        ] {
            let change = super::typed_mode_change(
                ModeAction::Add,
                mode,
                Some("alice"),
                kind,
                Casemapping::default(),
                origin,
                crate::UserHandle::from_nick,
            );
            match (expected, change.arg) {
                ("nick", Some(ModeArg::Member { user, .. })) => {
                    assert_eq!(user.nick().origin(), origin);
                }
                ("mask", Some(ModeArg::Mask(value))) => assert_eq!(value.origin(), origin),
                ("key", Some(ModeArg::Key(value))) => assert_eq!(value.expose(), "alice"),
                ("value", Some(ModeArg::Value(value))) => assert_eq!(value, "alice"),
                _ => panic!("unexpected typed mode argument"),
            }
        }
    }

    #[test]
    fn maps_topic_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::__topic_changed(papo_core::LineEnvelope::default(), Casemapping::default().channel("#papo"), Some("Rust 1.90 is out".into()), papo_core::Source::from_proto(papo_proto::Source::__user("alice", Some("~alice"), Some("staff.example")), Casemapping::default()), papo_core::ReplyToken::__unknown(crate::Origin::NONE), papo_core::Party::Other)),
            Event::TopicChanged(changed)
                if changed.channel == "#papo"
                    && changed.topic.as_deref() == Some("Rust 1.90 is out")
                    && changed.actor.nick().is_some_and(|nick| nick == "alice")
        ));
    }

    #[test]
    fn maps_state_limit_reached() {
        for (core, public) in [
            (StateLimit::Channels, crate::StateLimit::Channels),
            (StateLimit::Members, crate::StateLimit::Members),
            (StateLimit::ListEntries, crate::StateLimit::ListEntries),
            (StateLimit::ReadMarkers, crate::StateLimit::ReadMarkers),
            (
                StateLimit::MetadataTargets,
                crate::StateLimit::MetadataTargets,
            ),
            (StateLimit::MetadataKeys, crate::StateLimit::MetadataKeys),
            (
                StateLimit::IsupportTokens,
                crate::StateLimit::IsupportTokens,
            ),
            #[cfg(feature = "unstable")]
            (
                StateLimit::BouncerNetworks,
                crate::StateLimit::BouncerNetworks,
            ),
        ] {
            assert!(matches!(
                forwarded(papo_core::Event::__state_limit_reached(core)),
                Event::Warning(Warning::StateLimitReached { kind }) if kind == public
            ));
        }
    }

    #[cfg(feature = "unstable")]
    mod unstable {
        use super::*;
        use crate::event_impl::LineView;

        fn source_person(source: &Source) -> Vec<crate::UserHandle> {
            source.user().cloned().into_iter().collect()
        }

        fn conversation_person(conversation: &Conversation) -> Vec<crate::UserHandle> {
            match conversation {
                Conversation::Private(peer) => vec![peer.clone()],
                Conversation::Channel(_) => Vec::new(),
            }
        }

        fn subject_person(subject: &crate::Subject) -> Vec<crate::UserHandle> {
            match subject {
                crate::Subject::User(user) => vec![user.clone()],
                _ => Vec::new(),
            }
        }

        fn line_people(line: LineView<'_>) -> Vec<crate::UserHandle> {
            let message = |actor: &crate::UserHandle, conversation: &Conversation| {
                [vec![actor.clone()], conversation_person(conversation)].concat()
            };
            match line {
                LineView::Message(line) | LineView::Incomplete(Incomplete::Message(line)) => {
                    message(&line.actor, &line.conversation)
                }
                LineView::Notice(line) | LineView::Incomplete(Incomplete::Notice(line)) => {
                    message(&line.actor, &line.conversation)
                }
                LineView::CtcpQuery(line) => message(&line.actor, &line.conversation),
                LineView::CtcpReply(line) => message(&line.actor, &line.conversation),
                LineView::TagMsg(line) => message(&line.actor, &line.conversation),
                LineView::Reacted(line) => message(&line.actor, &line.conversation),
                LineView::TypingChanged(line) => message(&line.actor, &line.conversation),
                LineView::Redacted(line) => [
                    source_person(&line.actor),
                    conversation_person(&line.conversation),
                ]
                .concat(),
                LineView::ServerNotice(line) => source_person(&line.actor),
                LineView::Other(_) => Vec::new(),
            }
        }

        // Exhaustive on purpose, and compiled with the unstable variants: a new Event variant
        // fails to build here until it says which people it names, and every person a live
        // event names must be built from the identity recorded when its line arrived, which
        // the table below checks after a nickname handoff.
        fn people(event: &Event) -> Vec<crate::UserHandle> {
            match event {
                Event::Message(_)
                | Event::Notice(_)
                | Event::Incomplete(_)
                | Event::ServerNotice(_)
                | Event::Restricted(_)
                | Event::CtcpQuery(_)
                | Event::CtcpReply(_)
                | Event::Other(_)
                | Event::TagMsg(_)
                | Event::Reacted(_)
                | Event::TypingChanged(_)
                | Event::Redacted(_) => line_people(event.line_view().expect("a line event")),
                Event::BroadcastCtcp(event) => vec![event.actor.clone()],
                Event::BroadcastTagMsg(event) => vec![event.actor.clone()],
                Event::Joined(event) => vec![event.actor.clone()],
                Event::Parted(event) => vec![event.actor.clone()],
                Event::Quit(event) => vec![event.actor.clone()],
                Event::NickChanged(event) => vec![event.actor.clone()],
                Event::AccountChanged(event) => vec![event.actor.clone()],
                Event::AwayChanged(event) => vec![event.actor.clone()],
                Event::HostChanged(event) => vec![event.actor.clone()],
                Event::RealnameChanged(event) => vec![event.actor.clone()],
                Event::MonitorOnline(event) => vec![event.subject.clone()],
                Event::MonitorOffline(event) => vec![event.subject.clone()],
                Event::Kicked(event) => {
                    [source_person(&event.actor), vec![event.subject.clone()]].concat()
                }
                Event::Invited(event) => {
                    [source_person(&event.actor), vec![event.subject.clone()]].concat()
                }
                Event::TopicChanged(event) => source_person(&event.actor),
                Event::ChannelRenamed(event) => source_person(&event.actor),
                Event::ModeChanged(event) => [
                    source_person(&event.actor),
                    subject_person(&event.subject),
                    event
                        .changes
                        .iter()
                        .filter_map(|change| match &change.arg {
                            Some(crate::ModeArg::Member { user, .. }) => Some(user.clone()),
                            _ => None,
                        })
                        .collect(),
                ]
                .concat(),
                Event::MetadataChanged(event) => subject_person(&event.subject),
                Event::ReadMarkerChanged(event) => conversation_person(&event.conversation),
                Event::Lagged(event) => event
                    .gaps
                    .iter()
                    .flat_map(|gap| conversation_person(&gap.conversation))
                    .collect(),
                // Replayed history names people as they were then; its handles are unproven
                // by design, not built from a live arrival.
                Event::History(_)
                | Event::Connecting(_)
                | Event::Connected(_)
                | Event::Motd(_)
                | Event::StandardReply(_)
                | Event::AccountCreation(_)
                | Event::PersistenceChanged(_)
                | Event::BouncerNetworkChanged(_)
                | Event::Warning(_)
                | Event::CapsChanged(_)
                | Event::FeaturesChanged(_)
                | Event::CasemappingChanged(_)
                | Event::AutoJoinFailed(_)
                | Event::ChannelSynced(_)
                | Event::MembersRefreshed(_)
                | Event::Disconnected(_) => Vec::new(),
            }
        }

        #[test]
        fn every_person_a_late_mapped_event_names_is_who_they_were_at_arrival() {
            use super::identity::{joined_review, mapped, server};

            for line in [
                ":alice!~alice@cloak.example ACCOUNT alice",
                ":alice!~alice@cloak.example AWAY :Back in five",
                ":alice!~alice@cloak.example CHGHOST ~alice staff.example",
                ":alice!~alice@cloak.example SETNAME :Alice Liddell",
                ":alice!~alice@cloak.example MODE #review +v papo",
                ":alice!~alice@cloak.example MODE alice +i",
                ":papo!~papo@localhost MODE #review +o alice",
                ":alice!~alice@cloak.example TOPIC #review :Release 0.1 review",
                ":alice!~alice@cloak.example KICK #review papo :Flooding",
                ":alice!~alice@cloak.example INVITE papo #ops",
                ":alice!~alice@cloak.example RENAME #review #release :Release review",
                ":ergo.test 761 papo alice display-name * :Alice Liddell",
                "@msgid=RfcBTzyxY120010 :alice!~alice@cloak.example PRIVMSG papo :can you review 142?",
                "@msgid=RfcBTzyxY120011 :papo!~papo@localhost PRIVMSG alice :looking now",
                ":alice!~alice@cloak.example NOTICE papo :build 142 is green",
                ":alice!~alice@cloak.example PRIVMSG papo :\x01VERSION\x01",
                "@+example/status=ready :alice!~alice@cloak.example TAGMSG #review",
                "@+draft/react=👍;+draft/reply=RfcBTzyxY120010 :papo!~papo@localhost TAGMSG alice",
                ":alice!~alice@cloak.example REDACT papo RfcBTzyxY120010 :sent too early",
                "@+typing=active :alice!~alice@cloak.example TAGMSG papo",
            ] {
                let (mut session, shared) = joined_review();
                server(&mut session, &shared, line);
                let current = |nick: &str| {
                    crate::UserHandle::from_current_state(
                        session.state().casemapping().nick(nick),
                        Some(&shared),
                    )
                };
                let alice = current("alice");
                let own = current("papo");
                let arrived: Vec<_> = std::iter::from_fn(|| session.poll_event()).collect();
                server(
                    &mut session,
                    &shared,
                    ":alice!~alice@cloak.example QUIT :Quit: bye",
                );
                server(
                    &mut session,
                    &shared,
                    ":mallory!~mallory@users.example NICK alice",
                );
                while session.poll_event().is_some() {}

                let named: Vec<_> = arrived
                    .into_iter()
                    .filter_map(|event| mapped(&session, &shared, event))
                    .flat_map(|event| people(&event))
                    .collect();
                assert!(named.contains(&alice), "{line} named {named:?}");
                for person in named {
                    assert!(person == alice || person == own, "{line} named {person:?}");
                }
            }
        }

        #[test]
        fn people_are_a_line_actor_and_its_private_peer() {
            use super::identity::{joined_review, mapped, server};

            let (mut session, shared) = joined_review();
            server(
                &mut session,
                &shared,
                "@msgid=RfcBTzyxY120010 :alice!~alice@cloak.example PRIVMSG papo :can you review 142?",
            );
            let alice = crate::UserHandle::from_current_state(
                session.state().casemapping().nick("alice"),
                Some(&shared),
            );
            let event = session.poll_event().expect("private message");
            let event = mapped(&session, &shared, event).expect("mapped message");
            assert_eq!(people(&event), [alice.clone(), alice]);
        }

        // Exhaustive on purpose, and compiled with the unstable variants: a new Event variant
        // fails to build here until it is classified. A line that can carry a status sign must
        // also join `line_types!`, which the agreement test below checks on real lines.
        const fn carries_a_line(event: &Event) -> bool {
            match event {
                Event::Message(_)
                | Event::Notice(_)
                | Event::Incomplete(_)
                | Event::ServerNotice(_)
                | Event::Restricted(_)
                | Event::CtcpQuery(_)
                | Event::CtcpReply(_)
                | Event::Other(_)
                | Event::TagMsg(_)
                | Event::Reacted(_)
                | Event::TypingChanged(_)
                | Event::Redacted(_) => true,
                Event::Connecting(_)
                | Event::Connected(_)
                | Event::Lagged(_)
                | Event::Motd(_)
                | Event::BroadcastCtcp(_)
                | Event::BroadcastTagMsg(_)
                | Event::StandardReply(_)
                | Event::AccountCreation(_)
                | Event::PersistenceChanged(_)
                | Event::BouncerNetworkChanged(_)
                | Event::MetadataChanged(_)
                | Event::History(_)
                | Event::ReadMarkerChanged(_)
                | Event::Warning(_)
                | Event::CapsChanged(_)
                | Event::FeaturesChanged(_)
                | Event::CasemappingChanged(_)
                | Event::AutoJoinFailed(_)
                | Event::Joined(_)
                | Event::ChannelSynced(_)
                | Event::MembersRefreshed(_)
                | Event::Parted(_)
                | Event::Kicked(_)
                | Event::Invited(_)
                | Event::Quit(_)
                | Event::NickChanged(_)
                | Event::ChannelRenamed(_)
                | Event::AccountChanged(_)
                | Event::AwayChanged(_)
                | Event::HostChanged(_)
                | Event::RealnameChanged(_)
                | Event::ModeChanged(_)
                | Event::TopicChanged(_)
                | Event::MonitorOnline(_)
                | Event::MonitorOffline(_)
                | Event::Disconnected(_) => false,
            }
        }

        #[test]
        fn every_line_variant_is_separated_by_status() {
            let now = Instant::now();
            for line in [
                ":alice!~alice@cloak.example PRIVMSG #review :release approved",
                ":alice!~alice@cloak.example NOTICE #review :maintenance begins soon",
                ":irc.sol.test NOTICE #review :KNOCK: #review (bob[~bob@127.0.0.1] has asked for an invite)",
                ":alice!~alice@cloak.example PRIVMSG #review :\x01VERSION\x01",
                ":alice!~alice@cloak.example NOTICE #review :\x01VERSION Ergo-2.15.0\x01",
                "@+example/status=ready :alice!~alice@cloak.example TAGMSG #review",
                "@+draft/react=👍;+draft/reply=RfcBTzyxY120001 :alice!~alice@cloak.example TAGMSG #review",
                ":alice!~alice@cloak.example REDACT #review RfcBTzyxY120001 :off-topic",
                ":ergo.test 999 papo :an unknown numeric",
                ":alice!~alice@cloak.example JOIN #review",
                ":oper!staff@staff.example PRIVMSG $*.example.org :\x01VERSION\x01",
                ":alice!~alice@cloak.example NICK alicia",
            ] {
                let event = mapped_live_event(line, now);
                assert_eq!(
                    event.line_view().is_some(),
                    carries_a_line(&event),
                    "{line} -> {event:?}"
                );
            }
        }

        #[test]
        fn maps_request_untracked() {
            let now = Instant::now();
            let mut session = Session::new(
                Config::default()
                    .enable(papo_core::Feature::Persistence)
                    .wanted_caps(["draft/persistence"]),
            );
            session.handle_connected(now);
            session.handle_line(now, b"CAP * LS :draft/persistence");
            session.handle_line(now, b"CAP * ACK :draft/persistence");
            while session.poll_transmit().is_some() {}
            session.persistence_get(now).unwrap();
            assert!(matches!(
                from_core(session.poll_event().unwrap()),
                CoreEvent::RequestUntracked(_)
            ));
        }

        #[test]
        fn maps_bouncer_network_updates() {
            let event = forwarded(papo_core::Event::__bouncer_network_changed(
                papo_core::NetworkId::from("42"),
                None,
            ));
            let changed = event.as_bouncer_network_changed().unwrap();
            assert_eq!(changed.id, "42");
            assert!(changed.network.is_none());
        }
    }

    #[test]
    fn monitor_online_carries_only_handle() {
        assert!(matches!(
            forwarded(papo_core::Event::__monitor_online(Casemapping::default().nick("alice"), Some("~alice".into()), Some("cloak.example".into()), papo_core::ReplyToken::__unknown(crate::Origin::NONE))),
            Event::MonitorOnline(online)
                if online.subject.nick() == "alice"
        ));
        assert!(matches!(
            forwarded(papo_core::Event::__monitor_online(Casemapping::default().nick("bob"), None, None, papo_core::ReplyToken::__unknown(crate::Origin::NONE))),
            Event::MonitorOnline(online)
                if online.subject.nick() == "bob"
        ));
    }

    #[test]
    fn maps_monitor_offline() {
        assert!(matches!(
            forwarded(papo_core::Event::__monitor_offline(
                Casemapping::default().nick("alice"),
                papo_core::ReplyToken::__unknown(papo_proto::Origin::NONE),
                None,
            )),
            Event::MonitorOffline(offline)
                if offline.subject == "alice" && offline.facts.is_none()
        ));
    }

    #[test]
    fn maps_monitor_list_full() {
        assert!(matches!(
            forwarded(papo_core::Event::__monitor_list_full(100, vec![
                    Casemapping::default().nick("alice"),
                    Casemapping::default().nick("bob"),
                ])),
            Event::Warning(Warning::MonitorListFull { maximum, nicks })
                if maximum == 100 && nicks == ["alice", "bob"]
        ));
    }

    #[test]
    fn maps_unsupported_warning() {
        let unsupported =
            papo_core::Unsupported::new(Feature::Monitor, papo_core::UnsupportedReason::LimitZero);
        assert!(matches!(
            forwarded(papo_core::Event::Unsupported(unsupported)),
            Event::Warning(Warning::Unsupported(mapped)) if mapped == unsupported
        ));
    }

    #[test]
    fn maps_utf8_casemapping_warning() {
        assert!(matches!(
            forwarded(papo_core::Event::Utf8CasemappingUnsupported),
            Event::Warning(Warning::Utf8CasemappingUnsupported)
        ));
    }

    #[test]
    #[allow(clippy::duration_suboptimal_units)]
    fn maps_recreated_channel_warning() {
        let previous = crate::ServerTime::from_stored_millis(1_793_098_800_000).unwrap();
        let created = crate::ServerTime::from_stored_millis(1_793_099_900_000).unwrap();
        assert!(matches!(
            forwarded(papo_core::Event::__channel_recreated(
                Casemapping::default().channel("#release"),
                previous,
                created,
            )),
            Event::Warning(Warning::ChannelRecreated {
                channel,
                previous_created_at,
                created_at,
            }) if channel == "#release"
                && previous_created_at == previous
                && created_at == created
        ));
    }

    #[test]
    fn maps_history() {
        let message = Line::privmsg("#papo", "earlier");
        let core_conversation =
            papo_core::Conversation::Channel(Casemapping::Rfc1459.channel("#papo"));
        let CoreEvent::History {
            conversation: mapped,
            items,
            end,
        } = from_core(papo_core::Event::__history(
            core_conversation.clone(),
            vec![
                papo_core::HistoryItem::Message(papo_core::HistoryLine {
                    line: message.clone(),
                    context: false,
                    actor_party: papo_core::Party::Other,
                    subject_party: papo_core::Party::Unknown,
                }),
                papo_core::HistoryItem::Other(papo_core::HistoryLine {
                    line: Line::new("WALLOPS").with_params(["Network maintenance begins soon"]),
                    context: false,
                    actor_party: papo_core::Party::Unknown,
                    subject_party: papo_core::Party::Unknown,
                }),
            ],
            true,
        ))
        else {
            panic!("history was not classified")
        };
        assert_eq!(mapped, core_conversation);
        assert_eq!(items.len(), 2);
        assert!(matches!(
            &items[0],
            papo_core::HistoryItem::Message(item)
                if item.line == message && !item.context
        ));
        assert!(matches!(items[1], papo_core::HistoryItem::Other(_)));
        assert!(end);
    }

    #[test]
    fn keeps_history_targets_internal() {
        let targets = vec![(
            papo_core::Conversation::Channel(Casemapping::Rfc1459.channel("#papo")),
            crate::ServerTime::from_stored_millis(42_000).unwrap(),
        )];
        assert!(matches!(
            from_core(papo_core::Event::HistoryTargets(targets)),
            CoreEvent::Unknown
        ));
    }

    #[test]
    fn maps_read_markers() {
        let time = crate::ServerTime::from_stored_millis(1_760_529_342_123).unwrap();
        for (read_until, expected) in [
            (Some(time), crate::ReadMarker::ReadUntil(time)),
            (None, crate::ReadMarker::Unset),
        ] {
            let marker = papo_core::ReadMarker {
                conversation: papo_core::Conversation::Channel(
                    papo_proto::Casemapping::Rfc1459.channel("#papo"),
                ),
                read_until,
                peer_token: papo_core::ReplyToken::__unknown(crate::Origin::NONE),
            };
            assert!(matches!(
                forwarded(papo_core::Event::ReadMarkerChanged(marker.clone())),
                Event::ReadMarkerChanged(mapped)
                    if mapped.conversation == map_conversation(
                        marker.conversation.clone(),
                        crate::Origin::NONE,
                    )
                        && mapped.marker == expected
            ));
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn identifies_self_membership_and_nick_events_with_server_casemapping() {
        use crate::message_ref::Receipt;
        let mapping = Casemapping::Rfc1459;
        let actor = crate::__testing::source(papo_proto::Source::Server("ergo.test"), mapping);
        let own_handle = crate::__testing::user_handle(mapping.nick("Papo[Bot]"));
        let events = [
            Event::Joined(super::Joined {
                msgid: None,
                server_time: None,
                received_at: UNIX_EPOCH,
                channel: channel_handle(mapping.channel("#papo")),
                actor: own_handle.clone(),
                actor_party: Party::Me,
                netjoin: None,
            }),
            Event::Parted(super::Parted {
                msgid: None,
                server_time: None,
                received_at: UNIX_EPOCH,
                actor: own_handle.clone(),
                facts: None,
                channel: channel_handle(mapping.channel("#papo")),
                reason: None,
                actor_party: Party::Me,
            }),
            Event::Kicked(super::Kicked {
                msgid: None,
                server_time: None,
                received_at: UNIX_EPOCH,
                channel: channel_handle(mapping.channel("#papo")),
                subject: own_handle.clone(),
                facts: None,
                actor: actor.clone(),
                actor_party: Party::Other,
                subject_party: Party::Me,
                reason: None,
            }),
            Event::Invited(super::Invited {
                msgid: None,
                server_time: None,
                received_at: UNIX_EPOCH,
                channel: channel_handle(mapping.channel("#papo")),
                subject: own_handle.clone(),
                receipt: Receipt::without_recipient(
                    actor.clone(),
                    None,
                    actor.to_string(),
                    crate::Origin::NONE,
                ),
                actor,
                actor_party: Party::Other,
                subject_party: Party::Me,
            }),
            Event::NickChanged(super::NickChanged {
                msgid: None,
                server_time: None,
                received_at: UNIX_EPOCH,
                actor: own_handle.clone(),
                old: mapping.nick("Papo[Bot]"),
                new: mapping.nick("papo-away"),
                channels: vec![channel_handle(mapping.channel("#papo"))],
                actor_party: Party::Me,
            }),
            Event::Joined(super::Joined {
                msgid: None,
                server_time: None,
                received_at: UNIX_EPOCH,
                channel: channel_handle(mapping.channel("#papo")),
                actor: own_handle,
                actor_party: Party::Me,
                netjoin: Some(super::SplitServers {
                    batch: crate::BatchId::from("join"),
                    first: "irc.example".into(),
                    second: "irc.remote".into(),
                }),
            }),
        ];

        assert_eq!(events[0].as_joined().unwrap().actor_party, Party::Me);
        assert_eq!(events[1].as_parted().unwrap().actor_party, Party::Me);
        assert_eq!(events[2].as_kicked().unwrap().subject_party, Party::Me);
        assert_eq!(events[3].as_invited().unwrap().subject_party, Party::Me);
        assert_eq!(events[4].as_nick_changed().unwrap().actor_party, Party::Me);
        assert_eq!(events[5].as_joined().unwrap().actor_party, Party::Me);
        assert_eq!(
            events[5]
                .as_joined()
                .unwrap()
                .netjoin
                .as_ref()
                .unwrap()
                .batch,
            "join"
        );
    }

    #[test]
    fn maps_batch_abandoned() {
        for (core, public) in [
            (
                BatchAbandonReason::OpenBatchLimit,
                super::BatchAbandonReason::OpenBatchLimit,
            ),
            (
                BatchAbandonReason::LineLimit,
                super::BatchAbandonReason::LineLimit,
            ),
            (
                BatchAbandonReason::ByteLimit,
                super::BatchAbandonReason::ByteLimit,
            ),
            (
                BatchAbandonReason::Timeout,
                super::BatchAbandonReason::Timeout,
            ),
        ] {
            let Event::Warning(Warning::BatchAbandoned {
                id,
                batch_type,
                reason,
            }) = forwarded(papo_core::Event::__batch_abandoned(
                "history42".into(),
                "chathistory".into(),
                core,
            ))
            else {
                panic!("abandonment was not forwarded")
            };
            assert_eq!(id, "history42");
            assert_eq!(batch_type, "chathistory");
            assert_eq!(reason, public);
        }
    }

    mod identity {
        use std::{
            sync::{Arc, RwLock},
            time::{Instant, UNIX_EPOCH},
        };

        use papo_core::{Config, Session};

        use super::super::{CoreEvent, Event, from_core_with_receipt};
        use crate::UserHandle;

        struct Case {
            name: &'static str,
            line: &'static str,
            subject: &'static str,
            person: fn(&Event) -> Option<UserHandle>,
        }

        pub(super) type Shared = Arc<RwLock<Arc<crate::State>>>;

        pub(super) fn server(session: &mut Session, shared: &Shared, line: &str) {
            session.handle_line(Instant::now(), line.as_bytes());
            *shared.write().unwrap() = Arc::new(crate::State::from_session(session, true));
        }

        pub(super) fn joined_review() -> (Session, Shared) {
            let mut session = Session::new(Config::default().nick("papo"));
            session.monitor_add(["carol"]);
            let shared = Arc::new(RwLock::new(Arc::new(crate::State::from_session(
                &session, true,
            ))));
            for line in [
                ":ergo.test 005 papo CHANMODES=Ibe,k,fl,CEMRUimnstu CHANTYPES=# PREFIX=(qaohv)~&@%+ :are supported by this server",
                ":papo!~papo@localhost JOIN #review",
                ":alice!~alice@cloak.example JOIN #review",
                ":mallory!~mallory@users.example JOIN #review",
                ":dave!~dave@users.example JOIN #review",
            ] {
                server(&mut session, &shared, line);
            }
            while session.poll_event().is_some() {}
            (session, shared)
        }

        pub(super) fn mapped(
            session: &Session,
            shared: &Shared,
            event: papo_core::Event,
        ) -> Option<Event> {
            match from_core_with_receipt(
                event,
                session.isupport().casemapping(),
                session.state().nick().origin(),
                false,
                false,
                UNIX_EPOCH,
                Some(session.state()),
                Some(shared),
            ) {
                CoreEvent::Forward(event) => Some(event),
                CoreEvent::Line(super::super::CoreLine::Complete(line)) => {
                    Some(super::super::incoming_from_core(
                        line,
                        session.state().nick().origin(),
                        session.isupport(),
                        false,
                        false,
                        UNIX_EPOCH,
                        Some(shared),
                    ))
                }
                _ => None,
            }
        }

        fn assert_person_at_arrival(case: &Case) {
            let (mut session, shared) = joined_review();
            server(&mut session, &shared, case.line);
            let at_arrival = UserHandle::from_current_state(
                session.state().casemapping().nick(case.subject),
                Some(&shared),
            );
            server(
                &mut session,
                &shared,
                &format!(":{}!~alice@cloak.example QUIT :Quit: bye", case.subject),
            );
            server(
                &mut session,
                &shared,
                &format!(":mallory!~mallory@users.example NICK {}", case.subject),
            );
            let newcomer = UserHandle::from_current_state(
                session.state().casemapping().nick(case.subject),
                Some(&shared),
            );
            assert_ne!(at_arrival, newcomer, "{}: no handoff happened", case.name);

            let events: Vec<_> = std::iter::from_fn(|| session.poll_event()).collect();
            let person = events
                .into_iter()
                .filter_map(|event| mapped(&session, &shared, event))
                .find_map(|event| (case.person)(&event))
                .unwrap_or_else(|| panic!("{}: no event", case.name));
            assert_eq!(person, at_arrival, "{}", case.name);
            assert_eq!(person.nick(), case.subject, "{}", case.name);
        }

        #[test]
        fn presence_changes_name_the_person_at_arrival() {
            for case in [
                Case {
                    name: "account-notify",
                    line: ":alice!~alice@cloak.example ACCOUNT alice",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_account_changed()
                            .map(|changed| changed.actor.clone())
                    },
                },
                Case {
                    name: "away-notify",
                    line: ":alice!~alice@cloak.example AWAY :Back in five",
                    subject: "alice",
                    person: |event| event.as_away_changed().map(|changed| changed.actor.clone()),
                },
                Case {
                    name: "CHGHOST",
                    line: ":alice!~alice@cloak.example CHGHOST ~alice staff.example",
                    subject: "alice",
                    person: |event| event.as_host_changed().map(|changed| changed.actor.clone()),
                },
                Case {
                    name: "SETNAME",
                    line: ":alice!~alice@cloak.example SETNAME :Alice Liddell",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_realname_changed()
                            .map(|changed| changed.actor.clone())
                    },
                },
            ] {
                assert_person_at_arrival(&case);
            }
        }

        #[test]
        fn joins_and_renames_name_the_person_at_arrival() {
            for case in [
                Case {
                    name: "JOIN",
                    line: ":carol!~carol@cloak.example JOIN #review",
                    subject: "carol",
                    person: |event| {
                        event
                            .as_joined()
                            .filter(|joined| joined.actor.nick() == "carol")
                            .map(|joined| joined.actor.clone())
                    },
                },
                Case {
                    name: "NICK",
                    line: ":alice!~alice@cloak.example NICK alicia",
                    subject: "alicia",
                    person: |event| event.as_nick_changed().map(|changed| changed.actor.clone()),
                },
            ] {
                assert_person_at_arrival(&case);
            }
        }

        fn user_source(source: &crate::Source) -> Option<UserHandle> {
            source.user().cloned()
        }

        #[test]
        fn actors_and_subjects_name_the_person_at_arrival() {
            for case in [
                Case {
                    name: "KICK actor",
                    line: ":alice!~alice@cloak.example KICK #review dave :Flooding",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_kicked()
                            .and_then(|kicked| user_source(&kicked.actor))
                    },
                },
                Case {
                    name: "MODE actor",
                    line: ":alice!~alice@cloak.example MODE #review +v dave",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_mode_changed()
                            .and_then(|changed| user_source(&changed.actor))
                    },
                },
                Case {
                    name: "user MODE subject",
                    line: ":alice!~alice@cloak.example MODE alice +i",
                    subject: "alice",
                    person: |event| match event.as_mode_changed().map(|changed| &changed.subject) {
                        Some(crate::Subject::User(user)) => Some(user.clone()),
                        _ => None,
                    },
                },
                Case {
                    name: "INVITE actor",
                    line: ":alice!~alice@cloak.example INVITE papo #ops",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_invited()
                            .and_then(|invited| user_source(&invited.actor))
                    },
                },
                Case {
                    name: "INVITE subject",
                    line: ":dave!~dave@users.example INVITE alice #review",
                    subject: "alice",
                    person: |event| event.as_invited().map(|invited| invited.subject.clone()),
                },
                Case {
                    name: "TOPIC actor",
                    line: ":alice!~alice@cloak.example TOPIC #review :Release 0.1 review",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_topic_changed()
                            .and_then(|changed| user_source(&changed.actor))
                    },
                },
                Case {
                    name: "RENAME actor",
                    line: ":alice!~alice@cloak.example RENAME #review #release :Release review",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_channel_renamed()
                            .and_then(|renamed| user_source(&renamed.actor))
                    },
                },
            ] {
                assert_person_at_arrival(&case);
            }
        }

        #[test]
        fn metadata_and_monitor_name_the_person_at_arrival() {
            for case in [
                Case {
                    name: "METADATA",
                    line: ":ergo.test 761 papo alice display-name * :Alice Liddell",
                    subject: "alice",
                    person: |event| match event
                        .as_metadata_changed()
                        .map(|changed| &changed.subject)
                    {
                        Some(crate::Subject::User(user)) => Some(user.clone()),
                        _ => None,
                    },
                },
                Case {
                    name: "MONITOR online",
                    line: ":ergo.test 730 papo :carol!~carol@cloak.example",
                    subject: "carol",
                    person: |event| {
                        event
                            .as_monitor_online()
                            .map(|online| online.subject.clone())
                    },
                },
            ] {
                assert_person_at_arrival(&case);
            }
        }

        #[test]
        fn tags_redactions_and_typing_name_the_actor_at_arrival() {
            for case in [
                Case {
                    name: "reaction",
                    line: "@+draft/react=👍;+draft/reply=RfcBTzyxY120001 :alice!~alice@cloak.example TAGMSG #review",
                    subject: "alice",
                    person: |event| event.as_reacted().map(|reacted| reacted.actor.clone()),
                },
                Case {
                    name: "REDACT",
                    line: ":alice!~alice@cloak.example REDACT #review RfcBTzyxY120001 :off-topic",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_redacted()
                            .and_then(|redacted| user_source(&redacted.actor))
                    },
                },
                Case {
                    name: "typing stopped by the departure",
                    line: "@+typing=active :alice!~alice@cloak.example TAGMSG #review",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_typing_changed()
                            .filter(|change| change.state == crate::Typing::Done)
                            .map(|change| change.actor.clone())
                    },
                },
            ] {
                assert_person_at_arrival(&case);
            }
        }

        fn private_peer(conversation: &crate::Conversation) -> Option<UserHandle> {
            match conversation {
                crate::Conversation::Private(peer) => Some(peer.clone()),
                crate::Conversation::Channel(_) => None,
            }
        }

        #[test]
        fn private_peers_are_the_person_at_arrival() {
            for case in [
                Case {
                    name: "DM from alice",
                    line: "@msgid=RfcBTzyxY120010 :alice!~alice@cloak.example PRIVMSG papo :can you review 142?",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_message()
                            .and_then(|message| private_peer(&message.conversation))
                    },
                },
                Case {
                    name: "own DM to alice",
                    line: "@msgid=RfcBTzyxY120011 :papo!~papo@localhost PRIVMSG alice :looking now",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_message()
                            .and_then(|message| private_peer(&message.conversation))
                    },
                },
                Case {
                    name: "own private reaction",
                    line: "@+draft/react=👍;+draft/reply=RfcBTzyxY120010 :papo!~papo@localhost TAGMSG alice",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_reacted()
                            .and_then(|reacted| private_peer(&reacted.conversation))
                    },
                },
                Case {
                    name: "private redaction",
                    line: ":alice!~alice@cloak.example REDACT papo RfcBTzyxY120010 :sent too early",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_redacted()
                            .and_then(|redacted| private_peer(&redacted.conversation))
                    },
                },
                Case {
                    name: "private typing",
                    line: "@+typing=active :alice!~alice@cloak.example TAGMSG papo",
                    subject: "alice",
                    person: |event| {
                        event
                            .as_typing_changed()
                            .filter(|change| change.state == crate::Typing::Active)
                            .and_then(|change| private_peer(&change.conversation))
                    },
                },
            ] {
                assert_person_at_arrival(&case);
            }
        }

        #[test]
        fn an_own_private_message_is_sent_by_the_own_user() {
            let (mut session, shared) = joined_review();
            server(
                &mut session,
                &shared,
                "@msgid=RfcBTzyxY120011 :papo!~papo@localhost PRIVMSG alice :looking now",
            );
            let own = UserHandle::from_current_state(session.state().nick().clone(), Some(&shared));
            let events: Vec<_> = std::iter::from_fn(|| session.poll_event()).collect();
            let message = events
                .into_iter()
                .filter_map(|event| mapped(&session, &shared, event))
                .find_map(|event| event.as_message().cloned())
                .expect("own private message");
            assert_eq!(message.actor, own);
            assert_eq!(message.actor.nick(), "papo");
        }
    }
}
