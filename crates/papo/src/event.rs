use papo_core::{Account, Capabilities, RequestId, Response, Welcome, sts::StsAction};
use std::{fmt, time::SystemTime};

use papo_proto::{Ctcp, Isupport, Message, MessageRef, strip_formatting, time};

use crate::Client;

#[path = "event/channel.rs"]
mod channel;
#[path = "event/connection.rs"]
mod connection;
#[path = "event/conversation.rs"]
mod conversation;
#[path = "event/message_id.rs"]
mod message_id;
#[path = "event/raw.rs"]
mod raw;
#[path = "event/server.rs"]
mod server;
#[path = "event/source.rs"]
mod source;
#[path = "event/tags.rs"]
mod tags;
#[path = "event/user.rs"]
mod user;

pub use channel::{
    ChannelRenamed, ChannelSynced, Invited, Joined, Kicked, ModeChange, ModeChanged, Parted,
    TopicChanged,
};
pub use connection::{CapsChanged, Disconnected, Registered};
pub use conversation::{
    Reacted, ReadMarker, Redacted, StandardReply, StandardReplyKind, TagMsg, TypingChanged,
};
pub use message_id::TargetedMessageId;
pub use raw::RawLine;
pub use server::{
    BatchAbandonReason, MetadataChanged, PersistenceChanged, PersistenceSetting, StateLimit,
    Warning,
};
pub use source::Source;
pub use tags::Tags;
pub use user::{
    AccountChanged, AwayChanged, Departed, HostChanged, MonitorOffline, MonitorOnline, Netjoin,
    Netsplit, NickChanged, Quit, RealnameChanged,
};

/// The semantic kind of an incoming chat message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MessageKind {
    /// A regular `PRIVMSG`.
    Privmsg,
    /// A regular `NOTICE`.
    Notice,
    /// A CTCP `ACTION`.
    Action,
    /// Another CTCP command sent with `PRIVMSG`.
    Ctcp,
    /// A CTCP response sent with `NOTICE`.
    CtcpReply,
}

/// How an incoming message relates to the current session.
///
/// Replay detection is refined when typed batches and labeled requests are available.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Origin {
    /// A newly received message from another user.
    Live,
    /// An echo of a message sent by this client.
    ///
    /// Without labelled responses, messages from our nickname use this origin because the
    /// originating session cannot be identified.
    Echo,
    /// A message sent by another session of this account.
    OtherSession,
    /// A message associated with a server batch.
    Replay,
    /// A message returned by an explicit history request.
    History,
}

/// An incoming `PRIVMSG` or `NOTICE` with connection context.
#[derive(Clone)]
pub struct MessageEvent {
    client: Client,
    raw: Message,
    sender: Option<Source>,
    target: String,
    status: Option<String>,
    raw_text: String,
    text: String,
    tags: Tags,
    channel_context: Option<String>,
    private: bool,
    kind: StoredKind,
    isupport: Isupport,
    msgid: Option<String>,
    server_time: Option<SystemTime>,
    received_at: SystemTime,
    account: Option<String>,
    bot: bool,
    reply_to: Option<String>,
    relayed_by: Option<String>,
    // All three states distinguish absent, bare, and named protocol tags.
    #[allow(clippy::option_option)]
    oper: Option<Option<String>>,
    batch_type: Option<String>,
    origin: Origin,
    caps: Capabilities,
}

impl PartialEq for MessageEvent {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
            && self.sender == other.sender
            && self.target == other.target
            && self.status == other.status
            && self.raw_text == other.raw_text
            && self.text == other.text
            && self.tags == other.tags
            && self.channel_context == other.channel_context
            && self.private == other.private
            && self.kind == other.kind
            && self.isupport == other.isupport
            && self.msgid == other.msgid
            && self.server_time == other.server_time
            && self.received_at == other.received_at
            && self.account == other.account
            && self.bot == other.bot
            && self.reply_to == other.reply_to
            && self.relayed_by == other.relayed_by
            && self.oper == other.oper
            && self.batch_type == other.batch_type
            && self.origin == other.origin
            && self.caps == other.caps
    }
}

impl Eq for MessageEvent {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StoredKind {
    Privmsg,
    Notice,
    Action,
    Ctcp,
    CtcpReply,
}

impl MessageEvent {
    pub(crate) fn new(
        raw: Message,
        isupport: &Isupport,
        nick: &str,
        client: Client,
        caps: &Capabilities,
        received_at: SystemTime,
    ) -> Result<Self, Message> {
        let echo = raw.as_ref().source().is_some_and(|source| {
            let papo_proto::Source::User { nick: source, .. } = source else {
                return false;
            };
            isupport.casemapping().eq(source, nick)
        });
        let other_session = echo && caps.contains_key("labeled-response");
        Self::new_with_context(
            raw,
            None,
            echo && !other_session,
            other_session,
            isupport,
            client,
            caps,
            received_at,
        )
    }

    pub(crate) fn from_core(
        received: papo_core::ReceivedMessage,
        isupport: &Isupport,
        client: Client,
        caps: &Capabilities,
        received_at: SystemTime,
    ) -> Result<Self, Message> {
        let (raw, batch_type, echo, other_session) = received.into_context_parts();
        Self::new_with_context(
            raw,
            batch_type,
            echo,
            other_session,
            isupport,
            client,
            caps,
            received_at,
        )
    }

    pub(crate) fn from_history(
        message: Message,
        client: Client,
        received_at: SystemTime,
    ) -> Result<Self, Message> {
        Self::from_historical(message, client, received_at, Origin::History)
    }

    pub(crate) fn from_replay(
        message: Message,
        client: Client,
        received_at: SystemTime,
    ) -> Result<Self, Message> {
        Self::from_historical(message, client, received_at, Origin::Replay)
    }

    fn from_historical(
        message: Message,
        client: Client,
        received_at: SystemTime,
        origin: Origin,
    ) -> Result<Self, Message> {
        let snapshot = client.state();
        let mut event = Self::new_with_context(
            message,
            None,
            false,
            false,
            snapshot.isupport(),
            client,
            snapshot.caps(),
            received_at,
        )?;
        event.origin = origin;
        Ok(event)
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_context(
        raw: Message,
        batch_type: Option<String>,
        echo: bool,
        other_session: bool,
        isupport: &Isupport,
        client: Client,
        caps: &Capabilities,
        received_at: SystemTime,
    ) -> Result<Self, Message> {
        let message = raw.as_ref();
        let Some((raw_target, raw_text)) = message.param(0).zip(message.param(1)) else {
            return Err(raw);
        };
        let plain = if message.command_is("PRIVMSG") {
            StoredKind::Privmsg
        } else if message.command_is("NOTICE") {
            StoredKind::Notice
        } else {
            return Err(raw);
        };
        let (kind, displayed) = match Ctcp::parse(raw_text) {
            Some(ctcp)
                if plain == StoredKind::Privmsg && ctcp.command.eq_ignore_ascii_case("ACTION") =>
            {
                (StoredKind::Action, ctcp.params.unwrap_or(""))
            }
            Some(_) if plain == StoredKind::Privmsg => (StoredKind::Ctcp, raw_text),
            Some(_) => (StoredKind::CtcpReply, raw_text),
            None => (plain, raw_text),
        };
        let text = strip_formatting(displayed).into_owned();
        let raw_text = raw_text.to_owned();
        let status_len = raw_target
            .chars()
            .take_while(|prefix| {
                isupport
                    .statusmsg()
                    .is_some_and(|set| set.contains(*prefix))
            })
            .map(char::len_utf8)
            .sum();
        let (status, target) = raw_target.split_at(status_len);
        let status = (!status.is_empty()).then(|| status.to_owned());
        let target = target.to_owned();
        let private = !isupport.is_channel(&target);
        let sender = message.source().map(Source::from_proto);
        let tags = Tags::from_message(&message);
        let channel_context = tag_owned(&message, &["+channel-context", "+draft/channel-context"]);
        let msgid = tag_owned(&message, &["msgid"]);
        let server_time = tag_owned(&message, &["time", "znc.in/server-time-iso"])
            .and_then(|value| time::parse(&value));
        let account = tag_owned(&message, &["account"]);
        let bot = tags.contains("bot");
        let reply_to = tag_owned(&message, &["+reply", "+draft/reply"]);
        let relayed_by = tag_owned(&message, &["draft/relaymsg"]);
        let oper = message
            .tags()
            .find(|(key, _)| *key == "draft/oper")
            .map(|(_, value)| value.map(std::borrow::Cow::into_owned));
        let replay = batch_type.as_deref().is_some_and(|kind| {
            matches!(
                kind,
                "chathistory"
                    | "draft/chathistory"
                    | "draft/chathistory-targets"
                    | "draft/event-playback"
            )
        });
        let origin = if echo {
            Origin::Echo
        } else if other_session {
            Origin::OtherSession
        } else if replay {
            Origin::Replay
        } else {
            Origin::Live
        };
        Ok(Self {
            client,
            raw,
            sender,
            target,
            status,
            raw_text,
            text,
            tags,
            channel_context,
            private,
            kind,
            isupport: isupport.clone(),
            msgid,
            server_time,
            received_at,
            account,
            bot,
            reply_to,
            relayed_by,
            oper,
            batch_type,
            origin,
            caps: caps.clone(),
        })
    }

    pub(crate) fn sender_nick(&self) -> Option<&str> {
        match self.sender.as_ref()? {
            Source::User { nick, .. } => Some(nick),
            Source::Server { .. } => None,
        }
    }

    pub(crate) fn channel_context(&self) -> Option<&str> {
        self.channel_context.as_deref()
    }

    /// Returns the semantic kind of this message.
    #[must_use]
    pub const fn kind(&self) -> MessageKind {
        match &self.kind {
            StoredKind::Privmsg => MessageKind::Privmsg,
            StoredKind::Notice => MessageKind::Notice,
            StoredKind::Action => MessageKind::Action,
            StoredKind::Ctcp => MessageKind::Ctcp,
            StoredKind::CtcpReply => MessageKind::CtcpReply,
        }
    }

    /// Returns the display text with IRC formatting removed.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the text parameter exactly as received.
    #[must_use]
    pub fn raw_text(&self) -> &str {
        &self.raw_text
    }

    /// Returns all message tags in wire order.
    #[must_use]
    pub const fn tags(&self) -> &Tags {
        &self.tags
    }

    /// Returns the parsed source, when the server supplied one.
    #[must_use]
    pub fn source(&self) -> Option<Source> {
        self.sender.clone()
    }

    /// Returns the message target.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Reports whether the target is not a channel under the captured `CHANTYPES` value.
    #[must_use]
    pub fn is_private(&self) -> bool {
        self.private
    }

    /// Returns the channel target, or the sender for a private message.
    #[must_use]
    pub fn reply_target(&self) -> &str {
        if self.is_private()
            && let Some(Source::User { nick, .. }) = &self.sender
        {
            nick
        } else {
            self.target()
        }
    }

    /// Returns the server-assigned message identifier, when present.
    #[must_use]
    pub fn msgid(&self) -> Option<&str> {
        self.msgid.as_deref()
    }

    /// Returns the targeted message identifier, when the server supplied one.
    ///
    /// `None` means the message had no `msgid` tag.
    #[must_use]
    pub fn id(&self) -> Option<TargetedMessageId> {
        self.msgid()
            .map(|msgid| TargetedMessageId::new(self.reply_target(), msgid))
    }

    /// Returns server time when valid, or the local receipt time.
    #[must_use]
    pub fn time(&self) -> SystemTime {
        self.server_time.unwrap_or(self.received_at)
    }

    /// Returns the sender's authenticated account, when tagged.
    #[must_use]
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    /// Reports whether the message carried the `bot` tag.
    #[must_use]
    pub fn is_bot(&self) -> bool {
        self.bot
    }

    /// Returns the message identifier referenced by a reply tag.
    #[must_use]
    pub fn reply_to(&self) -> Option<&str> {
        self.reply_to.as_deref()
    }

    /// Returns the nickname of the client that submitted a relayed message.
    #[must_use]
    pub fn relayed_by(&self) -> Option<&str> {
        self.relayed_by.as_deref()
    }

    /// Returns whether the sender is an IRC operator and their oper name when supplied.
    #[must_use]
    #[allow(clippy::option_option)]
    pub fn oper(&self) -> Option<Option<&str>> {
        self.oper.as_ref().map(|name| name.as_deref())
    }

    /// Returns the containing batch type, when known.
    #[must_use]
    pub fn batch_type(&self) -> Option<&str> {
        self.batch_type.as_deref()
    }

    /// Returns how this message relates to the current session.
    #[must_use]
    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// Reports whether a bot may treat this as a newly received message.
    ///
    /// Messages from another session of our account are not live, so bots do not answer them.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.origin == Origin::Live
    }

    /// Reports whether this is an echo of a message sent by this client.
    #[must_use]
    pub fn is_own(&self) -> bool {
        self.origin == Origin::Echo
    }

    /// Parses the raw text as CTCP when it contains a CTCP command.
    ///
    /// `None` means the message text is not valid CTCP.
    #[must_use]
    pub fn ctcp(&self) -> Option<Ctcp<'_>> {
        Ctcp::parse(&self.raw_text)
    }

    /// Borrows the underlying IRC message.
    #[must_use]
    pub fn as_ref(&self) -> papo_proto::MessageRef<'_> {
        self.raw.as_ref()
    }
}

impl fmt::Debug for MessageEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MessageEvent")
            .field("client", &self.client)
            .field("raw", &self.raw)
            .finish_non_exhaustive()
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

fn tag_owned(message: &MessageRef<'_>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| message.tag(key).map(std::borrow::Cow::into_owned))
}

/// An event emitted by an IRC client.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// Connection registration completed.
    Registered(Registered),
    /// An incoming `PRIVMSG` or `NOTICE`.
    Message(Box<MessageEvent>),
    /// A parsed message without a typed event variant.
    ///
    /// A minor release may start typing a command, which then arrives as its typed variant.
    /// Match on `Raw` only for display and logging.
    Raw(RawLine),
    /// A tag-only message with typed accessors for known client tags.
    TagMsg(TagMsg),
    /// A tag-only reaction or unreaction.
    Reacted(Reacted),
    /// A tag-only typing notification.
    TypingChanged(TypingChanged),
    /// A message was redacted from a target's history.
    Redacted(Redacted),
    /// An unsolicited `IRCv3` standard reply.
    StandardReply(StandardReply),
    /// Account creation completed and the connection is authenticated.
    AccountRegistration(crate::AccountRegistration),
    /// The server reported the account's persistence state.
    PersistenceChanged(PersistenceChanged),
    /// A target's cached metadata changed.
    MetadataChanged(MetadataChanged),
    /// A completed history or playback batch.
    History(Box<crate::HistoryPage>),
    /// A target's read marker advanced or was first reported.
    ReadMarker(ReadMarker),
    /// Users departed because connectivity between two servers was lost.
    ///
    /// Non-`QUIT` lines in the batch are delivered as separate [`Event::Message`] values.
    Netsplit(Netsplit),
    /// Users returned after connectivity between two servers was restored.
    ///
    /// Non-`JOIN` lines in the batch are delivered as separate [`Event::Message`] values.
    Netjoin(Netjoin),
    /// A server batch exceeded a retention bound or deadline.
    Warning(Warning),
    /// The capabilities enabled on an established connection changed.
    CapsChanged(CapsChanged),
    /// A user joined a channel shared with the client.
    Joined(Joined),
    /// Initial channel membership synchronization completed.
    ChannelSynced(ChannelSynced),
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
    /// A channel topic changed live.
    TopicChanged(TopicChanged),
    /// Server input exceeded a configured state-retention bound.
    /// A monitored nickname came online or changed its userhost.
    MonitorOnline(MonitorOnline),
    /// A monitored nickname went offline.
    MonitorOffline(MonitorOffline),
    /// The server rejected nicknames because its monitor list was full.
    /// The connection ended.
    Disconnected(Disconnected),
}

macro_rules! event_accessors {
    () => {};
    ((boxed $variant:ident, $method:ident, $payload:ty), $($rest:tt)*) => {
        impl Event {
            #[doc = concat!("Returns the payload when this is [`Event::", stringify!($variant), "`].")]
            #[must_use]
            pub fn $method(&self) -> Option<&$payload> {
                if let Self::$variant(payload) = self {
                    Some(payload)
                } else {
                    None
                }
            }
        }
        event_accessors! { $($rest)* }
    };
    (($variant:ident, $method:ident, $payload:ty), $($rest:tt)*) => {
        impl Event {
            #[doc = concat!("Returns the payload when this is [`Event::", stringify!($variant), "`].")]
            #[must_use]
            pub const fn $method(&self) -> Option<&$payload> {
                if let Self::$variant(payload) = self {
                    Some(payload)
                } else {
                    None
                }
            }
        }
        event_accessors! { $($rest)* }
    };
}

event_accessors! {
    (boxed Message, as_message, MessageEvent),
    (boxed History, as_history, crate::HistoryPage),
    (Registered, as_registered, Registered),
    (Disconnected, as_disconnected, Disconnected),
    (CapsChanged, as_caps_changed, CapsChanged),
    (Reacted, as_reacted, Reacted),
    (Redacted, as_redacted, Redacted),
    (TypingChanged, as_typing_changed, TypingChanged),
    (TagMsg, as_tag_msg, TagMsg),
    (ReadMarker, as_read_marker, ReadMarker),
    (StandardReply, as_standard_reply, StandardReply),
    (Joined, as_joined, Joined),
    (ChannelSynced, as_channel_synced, ChannelSynced),
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
    (Netsplit, as_netsplit, Netsplit),
    (Netjoin, as_netjoin, Netjoin),
    (MetadataChanged, as_metadata_changed, MetadataChanged),
    (PersistenceChanged, as_persistence_changed, PersistenceChanged),
    (AccountRegistration, as_account_registration, crate::AccountRegistration),
    (Warning, as_warning, Warning),
    (Raw, as_raw, RawLine),
}

impl Event {
    pub(crate) fn mark_self(&mut self, nick: &str, casemapping: papo_proto::Casemapping) {
        let candidate = match self {
            Self::Joined(joined) => Some((&joined.nick, &mut joined.is_self)),
            Self::Parted(parted) => Some((&parted.nick, &mut parted.is_self)),
            Self::Kicked(kicked) => Some((&kicked.nick, &mut kicked.is_self)),
            Self::Invited(invited) => Some((&invited.nick, &mut invited.is_self)),
            Self::NickChanged(changed) => Some((&changed.old, &mut changed.is_self)),
            _ => None,
        };
        if let Some((candidate, is_self)) = candidate {
            *is_self = casemapping.eq(candidate, nick);
        }
        if let Self::Netjoin(netjoin) = self {
            for joined in &mut netjoin.joins {
                joined.is_self = casemapping.eq(&joined.nick, nick);
            }
        }
    }
}

// This transient mapping keeps forwarding allocation-free for every event kind.
#[allow(clippy::large_enum_variant)]
pub(crate) enum CoreEvent {
    Forward(Event),
    Registered(Welcome),
    Message(papo_core::ReceivedMessage),
    Raw(Message),
    Response(Response),
    RequestUntracked(RequestId),
    RequestTimedOut(RequestId),
    RequestAborted(RequestId),
    StsUpgrade(u16),
    StsPolicy(StsAction),
    History {
        target: String,
        messages: Vec<papo_core::HistoryMessage>,
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

// Keeping the exhaustive forwarding table together makes omissions visible when core grows.
#[allow(clippy::too_many_lines)]
pub(crate) fn from_core(event: papo_core::Event) -> CoreEvent {
    CoreEvent::Forward(match event {
        papo_core::Event::Registered(welcome) => return CoreEvent::Registered(welcome),
        papo_core::Event::Message(message) => {
            if message.as_ref().command_is("001") {
                return CoreEvent::Unknown;
            }
            return CoreEvent::Message(message);
        }
        papo_core::Event::MultilineMessage(message) => return CoreEvent::Raw(message.message),
        papo_core::Event::TagMessage(message) => {
            let tag = TagMsg {
                source: message.source.map(Source::from_prefix),
                target: message.target,
                tags: Tags::from_pairs(message.tags),
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
                    source: tag.source,
                    id: TargetedMessageId::new(tag.target, msgid),
                    reaction: reaction.to_owned(),
                    removed,
                })
            } else if let Some(state) = tag.tags.get("+typing").and_then(|state| match state {
                "active" => Some(crate::Typing::Active),
                "paused" => Some(crate::Typing::Paused),
                "done" => Some(crate::Typing::Done),
                _ => None,
            }) {
                Event::TypingChanged(TypingChanged {
                    source: tag.source,
                    target: tag.target,
                    state,
                })
            } else {
                Event::TagMsg(tag)
            }
        }
        papo_core::Event::Redacted(redacted) => Event::Redacted(Redacted {
            source: redacted.source.map(Source::from_prefix),
            id: TargetedMessageId::new(redacted.target, redacted.msgid),
            reason: redacted.reason,
        }),
        papo_core::Event::Response(response) => return CoreEvent::Response(response),
        papo_core::Event::RequestUntracked { id } => return CoreEvent::RequestUntracked(id),
        papo_core::Event::RequestTimedOut { id } => return CoreEvent::RequestTimedOut(id),
        papo_core::Event::RequestAborted { id } => return CoreEvent::RequestAborted(id),
        papo_core::Event::RequestUnsupported { id, unsupported } => {
            return CoreEvent::RequestUnsupported { id, unsupported };
        }
        papo_core::Event::StandardReply(reply) => Event::StandardReply(StandardReply {
            kind: match reply.kind {
                papo_proto::StandardReplyKind::Fail => StandardReplyKind::Fail,
                papo_proto::StandardReplyKind::Warn => StandardReplyKind::Warn,
                papo_proto::StandardReplyKind::Note => StandardReplyKind::Note,
                _ => return CoreEvent::Unknown,
            },
            command: reply.command,
            code: reply.code,
            context: reply.context,
            description: reply.description,
        }),
        papo_core::Event::AccountRegistered { account, .. } => {
            Event::AccountRegistration(crate::AccountRegistration::Registered { account })
        }
        papo_core::Event::VerificationRequired { account, message } => {
            Event::AccountRegistration(crate::AccountRegistration::VerificationRequired {
                account,
                message,
            })
        }
        papo_core::Event::PersistenceStatus { setting, effective } => {
            Event::PersistenceChanged(PersistenceChanged {
                setting: match setting {
                    papo_core::PersistenceSetting::On => PersistenceSetting::On,
                    papo_core::PersistenceSetting::Off => PersistenceSetting::Off,
                    papo_core::PersistenceSetting::Default => PersistenceSetting::Default,
                    _ => return CoreEvent::Unknown,
                },
                effective,
            })
        }
        papo_core::Event::MetadataChanged {
            target,
            key,
            value,
            visibility,
        } => Event::MetadataChanged(MetadataChanged {
            target,
            key,
            value,
            visibility,
        }),
        papo_core::Event::History { target, items, end } => {
            return CoreEvent::History {
                target,
                messages: items
                    .into_iter()
                    .filter_map(papo_core::HistoryItem::into_message)
                    .collect(),
                end,
            };
        }
        papo_core::Event::ReadMarker(marker) => Event::ReadMarker(ReadMarker {
            target: marker.target,
            time: marker.time,
        }),
        papo_core::Event::Netsplit { servers, quits } => Event::Netsplit(Netsplit {
            servers,
            quits: quits
                .into_iter()
                .filter_map(|message| {
                    let message = message.as_ref();
                    let nick = match message.source()? {
                        papo_proto::Source::User { nick, .. } => nick.to_owned(),
                        papo_proto::Source::Server(_) => return None,
                    };
                    Some(Departed {
                        nick,
                        reason: message.param(0).map(str::to_owned),
                    })
                })
                .collect(),
        }),
        papo_core::Event::Netjoin { servers, joins } => Event::Netjoin(Netjoin {
            servers,
            joins: joins
                .into_iter()
                .filter_map(|message| {
                    let message = message.as_ref();
                    let nick = match message.source()? {
                        papo_proto::Source::User { nick, .. } => nick.to_owned(),
                        papo_proto::Source::Server(_) => return None,
                    };
                    Some(Joined {
                        channel: message.param(0)?.to_owned(),
                        nick,
                        is_self: false,
                    })
                })
                .collect(),
        }),
        papo_core::Event::BatchAbandoned {
            reference,
            batch_type,
            reason,
        } => Event::Warning(Warning::BatchAbandoned {
            reference,
            batch_type,
            reason: match reason {
                papo_core::BatchAbandonReason::OpenBatchLimit => BatchAbandonReason::OpenBatchLimit,
                papo_core::BatchAbandonReason::LineLimit => BatchAbandonReason::LineLimit,
                papo_core::BatchAbandonReason::ByteLimit => BatchAbandonReason::ByteLimit,
                papo_core::BatchAbandonReason::Timeout => BatchAbandonReason::Timeout,
                _ => return CoreEvent::Unknown,
            },
        }),
        papo_core::Event::Malformed { line } => Event::Warning(Warning::Malformed { line }),
        papo_core::Event::Close { kind, reason } => return CoreEvent::Close { kind, reason },
        papo_core::Event::AuthenticationFailed { reason } => {
            Event::Warning(Warning::AuthenticationFailed { reason })
        }
        papo_core::Event::CapsChanged { added, removed } => {
            Event::CapsChanged(CapsChanged { added, removed })
        }
        papo_core::Event::StsUpgrade { port } => return CoreEvent::StsUpgrade(port),
        papo_core::Event::StsPolicy(action) => return CoreEvent::StsPolicy(action),
        papo_core::Event::Joined { channel, nick } => Event::Joined(Joined {
            channel,
            nick,
            is_self: false,
        }),
        papo_core::Event::ChannelSynced { channel } => {
            Event::ChannelSynced(ChannelSynced { channel })
        }
        papo_core::Event::Parted {
            channel,
            nick,
            reason,
        } => Event::Parted(Parted {
            channel,
            nick,
            reason,
            is_self: false,
        }),
        papo_core::Event::Kicked {
            channel,
            nick,
            by,
            reason,
        } => Event::Kicked(Kicked {
            channel,
            nick,
            by: Source::from_name(by),
            reason,
            is_self: false,
        }),
        papo_core::Event::Quit {
            nick,
            reason,
            channels,
        } => Event::Quit(Quit {
            nick,
            reason,
            channels,
        }),
        papo_core::Event::NickChanged { old, new } => Event::NickChanged(NickChanged {
            old,
            new,
            is_self: false,
        }),
        papo_core::Event::ChannelRenamed { old, new, reason } => {
            Event::ChannelRenamed(ChannelRenamed { old, new, reason })
        }
        papo_core::Event::AccountChanged { nick, account } => {
            Event::AccountChanged(AccountChanged {
                nick,
                account: match account {
                    Account::LoggedIn(account) => Some(account.into_string()),
                    _ => None,
                },
            })
        }
        papo_core::Event::AwayChanged { nick, message } => {
            Event::AwayChanged(AwayChanged { nick, message })
        }
        papo_core::Event::HostChanged { nick, user, host } => {
            Event::HostChanged(HostChanged { nick, user, host })
        }
        papo_core::Event::RealnameChanged { nick, realname } => {
            Event::RealnameChanged(RealnameChanged { nick, realname })
        }
        papo_core::Event::ModeChanged {
            target,
            by,
            changes,
        } => Event::ModeChanged(ModeChanged {
            target,
            by: Source::from_name(by),
            changes: changes
                .into_iter()
                .map(|change| ModeChange {
                    set: change.set,
                    mode: change.mode,
                    arg: change.arg,
                })
                .collect(),
        }),
        papo_core::Event::TopicChanged { channel, topic, by } => {
            Event::TopicChanged(TopicChanged {
                channel,
                topic,
                by: Source::from_name(by),
            })
        }
        papo_core::Event::StateLimitReached { limit } => {
            Event::Warning(Warning::StateLimitReached {
                limit: match limit {
                    papo_core::StateLimit::Channels => StateLimit::Channels,
                    papo_core::StateLimit::Members => StateLimit::Members,
                    papo_core::StateLimit::MetadataTargets => StateLimit::MetadataTargets,
                    papo_core::StateLimit::MetadataKeys => StateLimit::MetadataKeys,
                    _ => return CoreEvent::Unknown,
                },
            })
        }
        papo_core::Event::MonitorOnline { nick, user, host } => {
            Event::MonitorOnline(MonitorOnline { nick, user, host })
        }
        papo_core::Event::MonitorOffline { nick } => Event::MonitorOffline(MonitorOffline { nick }),
        papo_core::Event::MonitorListFull { limit, nicks } => {
            Event::Warning(Warning::MonitorListFull { limit, nicks })
        }
        papo_core::Event::MonitorUnsupported => Event::Warning(Warning::MonitorUnsupported),
        _ => return CoreEvent::Unknown,
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant, UNIX_EPOCH};

    use super::{
        CoreEvent, Event, MessageEvent, MessageKind, Origin, PersistenceSetting, Source,
        StandardReplyKind, Warning, from_core,
    };
    use papo_core::{
        Account, BatchAbandonReason, Capabilities, Config, Session,
        StandardReply as CoreStandardReply, StateLimit,
    };
    use papo_proto::{Isupport, Message, MessageRef, ModeKind, Typing};

    fn event(
        message: Message,
        isupport: &Isupport,
        nick: &str,
        received_at: std::time::SystemTime,
    ) -> MessageEvent {
        MessageEvent::new(
            message,
            isupport,
            nick,
            crate::Client::disconnected_for_test(),
            &Capabilities::new(),
            received_at,
        )
        .unwrap()
    }

    fn chat(command: &str, text: &str) -> MessageEvent {
        event(
            Message::new(command, ["#papo", text]).with_source("emily!~emily@localhost"),
            &Isupport::default(),
            "papo",
            UNIX_EPOCH,
        )
    }

    #[test]
    fn classifies_chat_and_ctcp_messages() {
        assert_eq!(chat("PRIVMSG", "hello").kind(), MessageKind::Privmsg);
        assert_eq!(chat("NOTICE", "hello").kind(), MessageKind::Notice);
        assert_eq!(
            chat("PRIVMSG", "\x01ACTION waves\x01").kind(),
            MessageKind::Action
        );
        assert_eq!(chat("PRIVMSG", "\x01VERSION\x01").kind(), MessageKind::Ctcp);
        assert_eq!(
            chat("PRIVMSG", "\x01VERSION\x01").ctcp().unwrap().command,
            "VERSION"
        );
        assert_eq!(
            chat("NOTICE", "\x01VERSION papo-0.1\x01").kind(),
            MessageKind::CtcpReply
        );
        assert!(
            MessageEvent::new(
                Message::new("FAIL", ["PRIVMSG", "\x01VERSION\x01"]),
                &Isupport::default(),
                "papo",
                crate::Client::disconnected_for_test(),
                &Capabilities::new(),
                UNIX_EPOCH,
            )
            .is_err()
        );
    }

    #[test]
    fn exposes_raw_and_display_text_once() {
        let formatted = chat("PRIVMSG", "hello \x02from Ergo\x02");
        assert_eq!(formatted.raw_text(), "hello \x02from Ergo\x02");
        assert_eq!(formatted.text(), "hello from Ergo");
        let action = chat("PRIVMSG", "\x01ACTION writes the specification\x01");
        assert_eq!(action.raw_text(), "\x01ACTION writes the specification\x01");
        assert_eq!(action.text(), "writes the specification");
    }

    #[test]
    fn debug_hides_connection_internals() {
        let client = crate::Client::disconnected_for_test();
        assert_eq!(format!("{client:?}"), "Client { .. }");

        let rendered = format!("{:?}", chat("PRIVMSG", "hello from Ergo"));
        assert!(rendered.contains("client: Client { .. }"));
        for field in ["commands:", "connected:", "isupport:", "caps:", "reply_to:"] {
            assert!(!rendered.contains(field), "{rendered}");
        }
    }

    #[test]
    fn routes_channels_and_private_messages_with_ergo_chantypes() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CHANTYPES=#"]);
        let channel = event(
            Message::privmsg("#papo", "hello").with_source("emily!~emily@localhost"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        assert!(!channel.is_private());
        assert_eq!(channel.target(), "#papo");
        assert_eq!(channel.reply_target(), "#papo");
        assert_eq!(
            channel.source(),
            Some(Source::User {
                nick: "emily".into(),
                user: Some("~emily".into()),
                host: Some("localhost".into())
            })
        );

        let private = event(
            Message::privmsg("papo", "hello").with_source("emily!~emily@localhost"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        assert!(private.is_private());
        assert_eq!(private.reply_target(), "emily");
    }

    #[test]
    fn separates_status_prefixes_from_the_channel_target() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CHANTYPES=#", "STATUSMSG=@+"]);
        let message = event(
            Message::privmsg("@+#papo", "operators and voiced members")
                .with_source("emily!~emily@localhost")
                .with_tag("bot", ""),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );

        assert_eq!(message.status.as_deref(), Some("@+"));
        assert_eq!(message.target(), "#papo");
        assert!(!message.is_private());
        assert_eq!(message.tags.get("bot"), Some(""));
    }

    #[test]
    fn exposes_message_tags_and_server_time() {
        let received = UNIX_EPOCH + Duration::from_secs(42);
        let tagged = event(
            Message::privmsg("#papo", "hello")
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
            tagged.time(),
            UNIX_EPOCH + Duration::new(1_770_091_506, 789_000_000)
        );
        assert_eq!(tagged.id().unwrap().target(), "#papo");
        assert_eq!(tagged.id().unwrap().msgid(), "RfcBTzyxY120004");
        assert_eq!(tagged.account(), Some("emily"));
        assert!(tagged.is_bot());
        assert_eq!(tagged.reply_to(), Some("RfcBTzyxY120003"));
        assert_eq!(tagged.relayed_by(), Some("bridge-bot"));
        assert!(!chat("PRIVMSG", "human message").is_bot());

        for time in [None, Some("not-a-time")] {
            let mut message = Message::privmsg("#papo", "fallback");
            if let Some(time) = time {
                message = message.with_tag("time", time);
            }
            assert_eq!(
                event(message, &Isupport::default(), "papo", received).time(),
                received
            );
        }
    }

    #[test]
    fn exposes_oper_tags_with_optional_names() {
        let tagged = |line| {
            event(
                MessageRef::parse(line).unwrap().to_owned(),
                &Isupport::default(),
                "papo",
                UNIX_EPOCH,
            )
        };
        assert_eq!(
            tagged(
                "@draft/oper=network-admin :alice!a@localhost PRIVMSG #papo :Scheduled maintenance"
            )
            .oper(),
            Some(Some("network-admin"))
        );
        assert_eq!(
            tagged("@draft/oper :alice!a@localhost PRIVMSG #papo :Scheduled maintenance").oper(),
            Some(None)
        );
        assert_eq!(chat("PRIVMSG", "ordinary message").oper(), None);
    }

    #[test]
    fn distinguishes_echoes_and_live_messages() {
        let classify = |message| event(message, &Isupport::default(), "Papo[bot]", UNIX_EPOCH);
        let echo = classify(Message::privmsg("#papo", "sent").with_source("papo{bot}!p@localhost"));
        assert_eq!(echo.origin(), Origin::Echo);
        assert!(!echo.is_live());
        let tagged = classify(
            Message::privmsg("#papo", "earlier")
                .with_source("emily!e@localhost")
                .with_tag("batch", "history42"),
        );
        assert_eq!(tagged.origin(), Origin::Live);
        assert!(tagged.is_live());
        let live = classify(Message::privmsg("#papo", "now").with_source("emily!e@localhost"));
        assert_eq!(live.origin(), Origin::Live);
        assert!(live.is_live());
    }
    fn forwarded(event: papo_core::Event) -> Event {
        let CoreEvent::Forward(event) = from_core(event) else {
            panic!("event was not forwarded")
        };
        event
    }

    fn mapped_message(event: papo_core::Event) -> MessageEvent {
        let CoreEvent::Message(message) = from_core(event) else {
            panic!("message was not mapped")
        };
        MessageEvent::from_core(
            message,
            &Isupport::default(),
            crate::Client::disconnected_for_test(),
            &Capabilities::new(),
            UNIX_EPOCH,
        )
        .unwrap()
    }

    fn batched_message(kind: &str, command: &str) -> super::MessageEvent {
        let now = Instant::now();
        let mut session = Session::new(Config::default().max_batch_lines(0));
        session.handle_line(now, format!("BATCH +b {kind} #papo").as_bytes());
        session.handle_line(now, format!("@batch=b :alice!~a@host {command}").as_bytes());
        let Some(core) = session.poll_event() else {
            panic!("expected batch fallback")
        };
        mapped_message(core)
    }

    fn labelled_session(now: Instant, config: Config) -> Session {
        let mut session = Session::new(config);
        session.handle_connected(now);
        session.handle_line(now, b"CAP * LS :labeled-response");
        session.handle_line(now, b"CAP * ACK :labeled-response");
        while session.poll_transmit().is_some() {}
        session
    }

    #[test]
    fn maps_response() {
        let now = Instant::now();
        let mut session = labelled_session(now, Config::default());
        let id = session
            .request(now, Message::new("MODE", ["#papo"]))
            .unwrap();
        session.handle_line(now, b"@label=0 :ergo.test ACK");
        assert!(
            matches!(from_core(session.poll_event().unwrap()), CoreEvent::Response(response) if response.id == id)
        );
    }

    #[test]
    fn maps_multiline_as_a_normal_message() {
        let multiline = papo_core::MultilineMessage {
            message: Message::privmsg("#papo", "first\nsecond"),
        };
        assert!(matches!(
            from_core(papo_core::Event::MultilineMessage(multiline)),
            CoreEvent::Raw(message) if message.as_ref().param(1) == Some("first\nsecond")
        ));
    }

    #[test]
    fn maps_tag_messages_with_typed_accessors() {
        let tag = papo_core::TagMessage {
            source: Some("alice!~a@cloak".into()),
            target: "#papo".into(),
            tags: vec![
                ("+typing".into(), Some("paused".into())),
                ("+draft/react".into(), Some("👍".into())),
                ("+reply".into(), Some("RKi9Cw".into())),
            ],
        };
        let Event::Reacted(reaction) = forwarded(papo_core::Event::TagMessage(tag)) else {
            panic!("expected reaction")
        };
        assert!(matches!(reaction.source, Some(Source::User { ref nick, .. }) if nick == "alice"));
        assert_eq!(reaction.id.target(), "#papo");
        assert_eq!(reaction.reaction, "👍");
        assert_eq!(reaction.id.msgid(), "RKi9Cw");
        assert!(!reaction.removed);
    }

    #[test]
    fn distinguishes_every_typing_tag_value() {
        let typing = |value: &str| {
            let tag = papo_core::TagMessage {
                source: None,
                target: "#papo".into(),
                tags: vec![("+typing".into(), Some(value.into()))],
            };
            match forwarded(papo_core::Event::TagMessage(tag)) {
                Event::TypingChanged(typing) => Some(typing.state),
                _ => None,
            }
        };
        assert_eq!(typing("active"), Some(Typing::Active));
        assert_eq!(typing("paused"), Some(Typing::Paused));
        assert_eq!(typing("done"), Some(Typing::Done));
        assert_eq!(typing("composing"), None);

        let tag = papo_core::TagMessage {
            source: Some("alice!~a@cloak".into()),
            target: "#papo".into(),
            tags: vec![("+typing".into(), Some("active".into()))],
        };
        assert!(
            matches!(forwarded(papo_core::Event::TagMessage(tag)), Event::TypingChanged(typing) if typing.state == Typing::Active)
        );
    }

    #[test]
    fn maps_redactions() {
        let redacted = papo_core::Redacted {
            source: Some("alice!~a@cloak".into()),
            target: "#papo".into(),
            msgid: "RKi9Cw".into(),
            reason: None,
        };
        assert!(matches!(
            forwarded(papo_core::Event::Redacted(redacted.clone())),
            Event::Redacted(mapped)
                if matches!(mapped.source, Some(Source::User { ref nick, .. }) if nick == "alice")
                    && mapped.id.target() == redacted.target
                    && mapped.id.msgid() == redacted.msgid
                    && mapped.reason == redacted.reason
        ));
    }

    #[test]
    fn maps_request_untracked() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().wanted_caps(["draft/persistence"]));
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
    fn maps_request_timed_out() {
        let now = Instant::now();
        let mut session = labelled_session(now, Config::default().request_timeout(Duration::ZERO));
        let id = session
            .request(now, Message::new("MODE", ["#papo"]))
            .unwrap();
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
            .request(now, Message::new("MODE", ["#papo"]))
            .unwrap();
        session.handle_disconnected(now);
        assert!(
            matches!(from_core(session.poll_event().unwrap()), CoreEvent::RequestAborted(mapped) if mapped == id)
        );
    }

    #[test]
    fn maps_standard_reply() {
        let reply = CoreStandardReply {
            kind: papo_proto::StandardReplyKind::Warn,
            command: "CHATHISTORY".into(),
            code: "RATE_LIMITED".into(),
            context: vec!["#papo".into()],
            description: "Please wait".into(),
        };
        let Event::StandardReply(mapped) =
            forwarded(papo_core::Event::StandardReply(reply.clone()))
        else {
            panic!("standard reply was not forwarded")
        };
        assert_eq!(mapped.kind, StandardReplyKind::Warn);
        assert_eq!(mapped.command, reply.command);
        assert_eq!(mapped.code, reply.code);
        assert_eq!(mapped.context, reply.context);
        assert_eq!(mapped.description, reply.description);
    }

    #[test]
    fn maps_account_registration_results() {
        assert!(matches!(
            forwarded(papo_core::Event::AccountRegistered {
                account: "alice".into(),
                logged_in: true,
            }),
            Event::AccountRegistration(crate::AccountRegistration::Registered { account })
                if account == "alice"
        ));
        assert!(matches!(
            forwarded(papo_core::Event::VerificationRequired {
                account: "alice".into(),
                message: "Verification code sent".into(),
            }),
            Event::AccountRegistration(crate::AccountRegistration::VerificationRequired {
                account,
                message,
            })
                if account == "alice" && message == "Verification code sent"
        ));
    }

    #[test]
    fn maps_channel_renames() {
        assert!(matches!(
            forwarded(papo_core::Event::ChannelRenamed {
                old: "#rust".into(),
                new: "#rust-lang".into(),
                reason: "Project renamed".into(),
            }),
            Event::ChannelRenamed(renamed)
                if renamed.old == "#rust"
                    && renamed.new == "#rust-lang"
                    && renamed.reason == "Project renamed"
        ));
    }

    #[test]
    fn maps_persistence_status() {
        assert!(matches!(
            forwarded(papo_core::Event::PersistenceStatus {
                setting: papo_core::PersistenceSetting::Default,
                effective: true,
            }),
            Event::PersistenceChanged(changed)
                if changed.setting == PersistenceSetting::Default && changed.effective
        ));
    }

    #[test]
    fn maps_metadata_events() {
        assert!(matches!(
            forwarded(papo_core::Event::MetadataChanged {
                target: "#papo".into(),
                key: "pronouns".into(),
                value: Some("they/them".into()),
                visibility: "members".into(),
            }),
            Event::MetadataChanged(changed)
                if changed.target == "#papo" && changed.key == "pronouns"
                    && changed.value.as_deref() == Some("they/them")
                    && changed.visibility == "members"
        ));
        assert!(matches!(
            from_core(papo_core::Event::MetadataSyncLater {
                target: "alice".into(),
                retry_after: Some(Duration::from_secs(90)),
            }),
            CoreEvent::Unknown
        ));
    }

    #[test]
    fn classifies_replay_origin_by_resolved_batch_type() {
        for kind in [
            "chathistory",
            "draft/chathistory",
            "draft/chathistory-targets",
            "draft/event-playback",
        ] {
            let replay = batched_message(kind, "PRIVMSG #papo :earlier");
            assert_eq!(replay.origin(), Origin::Replay);
            assert!(!replay.is_live());
        }

        for kind in ["netsplit", "vendor/sync"] {
            let live = batched_message(kind, "NOTICE papo :live batch");
            assert_eq!(live.batch_type(), Some(kind));
            assert_eq!(live.origin(), Origin::Live);
            assert!(live.is_live());
        }
    }

    #[test]
    fn explicit_history_has_a_non_live_origin() {
        let mut history = chat("PRIVMSG", "earlier");
        history.origin = Origin::History;
        assert_eq!(history.origin(), Origin::History);
        assert!(!history.is_live());
    }

    #[test]
    fn classifies_echo_before_batch_origin() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("Papo[bot]"));
        session.handle_line(now, b":papo{bot}!~p@host PRIVMSG #papo :sent");
        let echo = mapped_message(session.poll_event().expect("expected echo"));
        assert_eq!(echo.origin(), Origin::Echo);
        assert!(!echo.is_live());
    }

    #[test]
    fn maps_joined() {
        assert!(matches!(
            forwarded(papo_core::Event::Joined {
                channel: "#papo".into(),
                nick: "emily".into()
            }),
            Event::Joined(joined) if joined.channel == "#papo" && joined.nick == "emily"
        ));
    }

    #[test]
    fn maps_channel_synced() {
        assert!(matches!(
            forwarded(papo_core::Event::ChannelSynced {
                channel: "#papo".into()
            }),
            Event::ChannelSynced(synced) if synced.channel == "#papo"
        ));
    }

    #[test]
    fn maps_parted() {
        assert!(matches!(
            forwarded(papo_core::Event::Parted {
                channel: "#papo".into(),
                nick: "emily".into(),
                reason: Some("Leaving for lunch".into()),
            }),
            Event::Parted(parted)
                if parted.channel == "#papo"
                    && parted.nick == "emily"
                    && parted.reason.as_deref() == Some("Leaving for lunch")
        ));
    }

    #[test]
    fn maps_kicked() {
        assert!(matches!(
            forwarded(papo_core::Event::Kicked {
                channel: "#papo".into(),
                nick: "emily".into(),
                by: "alice".into(),
                reason: Some("Flooding".into()),
            }),
            Event::Kicked(kicked)
                if kicked.channel == "#papo"
                    && kicked.nick == "emily"
                    && kicked.by.nick() == Some("alice")
                    && kicked.reason.as_deref() == Some("Flooding")
        ));
    }

    #[test]
    fn maps_quit() {
        assert!(matches!(
            forwarded(papo_core::Event::Quit {
                nick: "emily".into(),
                reason: Some("Client exited".into()),
                channels: vec!["#papo".into(), "#rust".into()],
            }),
            Event::Quit(quit)
                if quit.nick == "emily" && quit.reason.as_deref() == Some("Client exited")
                    && quit.channels == ["#papo", "#rust"]
        ));
    }

    #[test]
    fn maps_nick_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::NickChanged {
                old: "emily".into(),
                new: "em".into(),
            }),
            Event::NickChanged(changed) if changed.old == "emily" && changed.new == "em"
        ));
    }

    #[test]
    fn maps_account_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::AccountChanged {
                nick: "emily".into(),
                account: Account::LoggedIn("emily".into()),
            }),
            Event::AccountChanged(changed)
                if changed.nick == "emily" && changed.account.as_deref() == Some("emily")
        ));
    }

    #[test]
    fn maps_away_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::AwayChanged {
                nick: "emily".into(),
                message: Some("Gone to lunch".into()),
            }),
            Event::AwayChanged(changed)
                if changed.nick == "emily" && changed.message.as_deref() == Some("Gone to lunch")
        ));
    }

    #[test]
    fn maps_host_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::HostChanged {
                nick: "emily".into(),
                user: "~em".into(),
                host: "users/emily".into(),
            }),
            Event::HostChanged(changed)
                if changed.nick == "emily" && changed.user == "~em" && changed.host == "users/emily"
        ));
    }

    #[test]
    fn maps_realname_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::RealnameChanged {
                nick: "emily".into(),
                realname: "Emily Smith".into(),
            }),
            Event::RealnameChanged(changed)
                if changed.nick == "emily" && changed.realname == "Emily Smith"
        ));
    }

    #[test]
    fn maps_mode_changed() {
        let mode_changes = vec![papo_core::OwnedModeChange {
            set: true,
            mode: 'o',
            arg: Some("emily".into()),
            kind: ModeKind::Prefix(0),
        }];
        assert!(matches!(
            forwarded(papo_core::Event::ModeChanged {
                target: "#papo".into(),
                by: "alice".into(),
                changes: mode_changes.clone(),
            }),
            Event::ModeChanged(mode)
                if mode.target == "#papo"
                    && mode.by.nick() == Some("alice")
                    && mode.changes.len() == 1
                    && mode.changes[0].set
                    && mode.changes[0].mode == 'o'
                    && mode.changes[0].arg.as_deref() == Some("emily")
        ));
    }

    #[test]
    fn maps_topic_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::TopicChanged {
                channel: "#papo".into(),
                topic: Some("Rust 1.90 is out".into()),
                by: "alice".into(),
            }),
            Event::TopicChanged(changed)
                if changed.channel == "#papo"
                    && changed.topic.as_deref() == Some("Rust 1.90 is out")
                    && changed.by.nick() == Some("alice")
        ));
    }

    #[test]
    fn maps_state_limit_reached() {
        assert!(matches!(
            forwarded(papo_core::Event::StateLimitReached {
                limit: StateLimit::Members
            }),
            Event::Warning(Warning::StateLimitReached {
                limit: super::StateLimit::Members
            })
        ));
    }

    #[test]
    fn maps_monitor_online_with_and_without_userhost() {
        assert!(matches!(
            forwarded(papo_core::Event::MonitorOnline {
                nick: "alice".into(),
                user: Some("~alice".into()),
                host: Some("cloak.example".into()),
            }),
            Event::MonitorOnline(online)
                if online.nick == "alice" && online.user.as_deref() == Some("~alice")
                    && online.host.as_deref() == Some("cloak.example")
        ));
        assert!(matches!(
            forwarded(papo_core::Event::MonitorOnline {
                nick: "bob".into(),
                user: None,
                host: None,
            }),
            Event::MonitorOnline(online)
                if online.nick == "bob" && online.user.is_none() && online.host.is_none()
        ));
    }

    #[test]
    fn maps_monitor_offline() {
        assert!(matches!(
            forwarded(papo_core::Event::MonitorOffline {
                nick: "alice".into(),
            }),
            Event::MonitorOffline(offline) if offline.nick == "alice"
        ));
    }

    #[test]
    fn maps_monitor_list_full() {
        assert!(matches!(
            forwarded(papo_core::Event::MonitorListFull {
                limit: 100,
                nicks: vec!["alice".into(), "bob".into()],
            }),
            Event::Warning(Warning::MonitorListFull { limit, nicks })
                if limit == 100 && nicks == ["alice", "bob"]
        ));
    }

    #[test]
    fn maps_monitor_unsupported() {
        assert!(matches!(
            forwarded(papo_core::Event::MonitorUnsupported),
            Event::Warning(Warning::MonitorUnsupported)
        ));
    }

    #[test]
    fn maps_history() {
        let message = Message::privmsg("#papo", "earlier");
        let CoreEvent::History {
            target,
            messages,
            end,
        } = from_core(papo_core::Event::History {
            target: "#papo".into(),
            items: vec![
                papo_core::HistoryItem::Message(papo_core::HistoryMessage {
                    message: message.clone(),
                    context: false,
                }),
                papo_core::HistoryItem::Other(papo_core::HistoryMessage {
                    message: Message::new("WALLOPS", ["Network maintenance begins soon"]),
                    context: false,
                }),
            ],
            end: true,
        })
        else {
            panic!("history was not classified")
        };
        assert_eq!(target, "#papo");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message, message);
        assert!(!messages[0].context);
        assert!(end);
    }

    #[test]
    fn keeps_history_targets_internal() {
        let targets = vec![("#papo".into(), UNIX_EPOCH + Duration::from_secs(42))];
        assert!(matches!(
            from_core(papo_core::Event::HistoryTargets(targets)),
            CoreEvent::Unknown
        ));
    }

    #[test]
    fn maps_read_markers() {
        let marker = papo_core::ReadMarker {
            target: "#papo".into(),
            time: Some(UNIX_EPOCH + Duration::from_secs(42)),
        };
        assert!(matches!(
            forwarded(papo_core::Event::ReadMarker(marker.clone())),
            Event::ReadMarker(mapped)
                if mapped.target == marker.target && mapped.time == marker.time
        ));
    }

    #[test]
    fn maps_netsplit() {
        let quit =
            Message::new("QUIT", ["irc.example irc.remote"]).with_source("alice!~a@remote.example");
        let Event::Netsplit(netsplit) = forwarded(papo_core::Event::Netsplit {
            servers: ["irc.example".into(), "irc.remote".into()],
            quits: vec![quit],
        }) else {
            panic!("netsplit was not forwarded")
        };
        assert_eq!(netsplit.servers, ["irc.example", "irc.remote"]);
        assert_eq!(netsplit.quits.len(), 1);
        assert_eq!(netsplit.quits[0].nick, "alice");
        assert_eq!(
            netsplit.quits[0].reason.as_deref(),
            Some("irc.example irc.remote")
        );
    }

    #[test]
    fn maps_netjoin() {
        let join = Message::join("#papo").with_source("alice!~a@remote.example");
        let Event::Netjoin(netjoin) = forwarded(papo_core::Event::Netjoin {
            servers: ["irc.example".into(), "irc.remote".into()],
            joins: vec![join],
        }) else {
            panic!("netjoin was not forwarded")
        };
        assert_eq!(netjoin.servers, ["irc.example", "irc.remote"]);
        assert_eq!(netjoin.joins.len(), 1);
        assert_eq!(netjoin.joins[0].channel, "#papo");
        assert_eq!(netjoin.joins[0].nick, "alice");
    }

    #[test]
    fn identifies_self_membership_and_nick_events_with_server_casemapping() {
        let by = Source::Server {
            name: "ergo.test".to_owned(),
        };
        let mut events = [
            Event::Joined(super::Joined {
                channel: "#papo".to_owned(),
                nick: "Papo[Bot]".to_owned(),
                is_self: false,
            }),
            Event::Parted(super::Parted {
                channel: "#papo".to_owned(),
                nick: "Papo[Bot]".to_owned(),
                reason: None,
                is_self: false,
            }),
            Event::Kicked(super::Kicked {
                channel: "#papo".to_owned(),
                nick: "Papo[Bot]".to_owned(),
                by: by.clone(),
                reason: None,
                is_self: false,
            }),
            Event::Invited(super::Invited {
                channel: "#papo".to_owned(),
                nick: "Papo[Bot]".to_owned(),
                by,
                is_self: false,
            }),
            Event::NickChanged(super::NickChanged {
                old: "Papo[Bot]".to_owned(),
                new: "papo-away".to_owned(),
                is_self: false,
            }),
            Event::Netjoin(super::Netjoin {
                servers: ["irc.example".to_owned(), "irc.remote".to_owned()],
                joins: vec![super::Joined {
                    channel: "#papo".to_owned(),
                    nick: "Papo[Bot]".to_owned(),
                    is_self: false,
                }],
            }),
        ];

        for event in &mut events {
            event.mark_self("papo{bot}", papo_proto::Casemapping::Rfc1459);
        }
        assert!(events[0].as_joined().unwrap().is_self);
        assert!(events[1].as_parted().unwrap().is_self);
        assert!(events[2].as_kicked().unwrap().is_self);
        assert!(events[3].as_invited().unwrap().is_self);
        assert!(events[4].as_nick_changed().unwrap().is_self);
        assert!(events[5].as_netjoin().unwrap().joins[0].is_self);

        events[0].mark_self("someone-else", papo_proto::Casemapping::Rfc1459);
        assert!(!events[0].as_joined().unwrap().is_self);
    }

    #[test]
    fn maps_batch_abandoned() {
        let Event::Warning(Warning::BatchAbandoned {
            reference,
            batch_type,
            reason,
        }) = forwarded(papo_core::Event::BatchAbandoned {
            reference: "history42".into(),
            batch_type: "chathistory".into(),
            reason: BatchAbandonReason::Timeout,
        })
        else {
            panic!("abandonment was not forwarded")
        };
        assert_eq!(reference, "history42");
        assert_eq!(batch_type, "chathistory");
        assert_eq!(reason, super::BatchAbandonReason::Timeout);
    }
}
