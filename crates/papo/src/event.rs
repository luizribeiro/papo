use papo_core::{Account, Conversation, RequestId, Response, Welcome, sts::StsAction};
use std::time::SystemTime;

#[cfg(feature = "unstable")]
use papo_proto::Nick;
use papo_proto::{
    AccountName, Casemapping, ChannelName, Ctcp as ProtoCtcp, Isupport, Line as ProtoLine, LineRef,
    MessageId, strip_formatting, time,
};

use crate::Client;

#[path = "event/channel.rs"]
mod channel;
#[path = "event/connection.rs"]
mod connection;
#[path = "event/conversation.rs"]
mod conversation;
#[path = "event/server.rs"]
mod server;
#[path = "event/source.rs"]
pub(crate) mod source;
#[path = "event/tags.rs"]
mod tags;
#[path = "event/user.rs"]
mod user;

pub use crate::metadata::Visibility;
pub use channel::{
    ChannelRenamed, ChannelSynced, Invited, Joined, Kicked, MembersRefreshed, ModeChanged, Parted,
    TopicChanged,
};
pub use connection::{
    CapsChanged, CasemappingChanged, Connected, Disconnected, FeaturesChanged, Motd,
};
pub use conversation::{
    Reacted, ReadMarkerChanged, Redacted, StandardReply, StandardReplyKind, TagMsg, TypingChanged,
};
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use papo_core::PersistenceSetting;
pub use papo_core::{BatchAbandonReason, StateLimit, TypingReason};
pub use papo_core::{ModeAction, ModeArg, ModeChange};
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use server::BouncerNetworkChanged;
pub use server::{MetadataChanged, Warning};
pub use source::Source;
pub use tags::Tags;
pub use user::{
    AccountChanged, AwayChanged, Departed, HostChanged, MonitorOffline, MonitorOnline, Netjoin,
    Netsplit, NickChanged, Quit, RealnameChanged, SplitServers,
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
}

/// An incoming Client-to-Client Protocol query or reply.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Ctcp {
    /// The user or server that sent the CTCP message.
    pub actor: Source,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// The IRC conversation to which the CTCP message belongs.
    pub conversation: Conversation,
    /// `STATUSMSG` prefixes removed from the wire address, in wire order.
    pub status: Option<Vec<char>>,
    /// The command with its original spelling.
    pub command: String,
    /// The command parameters, when present.
    pub params: Option<String>,
    /// Whether this is a response sent with `NOTICE`.
    pub reply: bool,
    /// The server-assigned message identifier, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, or the local receipt time when none was supplied.
    pub time: SystemTime,
    /// The actor's authenticated account, when tagged.
    pub account: Option<AccountName>,
    /// How the CTCP message relates to the current session.
    pub origin: Origin,
    /// All message tags in wire order.
    pub tags: Tags,
}

/// Whether an event's actor is this client's user.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Authorship {
    /// The actor is this client's user.
    Own,
    /// The actor is another user or a server.
    Other,
    /// The protocol did not prove who the actor is relative to this client.
    Unknown,
}

impl Authorship {
    pub(crate) const fn from_core(value: papo_core::Authorship) -> Self {
        match value {
            papo_core::Authorship::Own => Self::Own,
            papo_core::Authorship::Other => Self::Other,
            _ => Self::Unknown,
        }
    }
}

/// How an incoming message relates to the current session.
///
/// Replay detection is refined when typed batches and labeled requests are available.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Origin {
    /// A newly received message.
    Live,
    /// A message associated with a server batch.
    Replay,
    /// A message returned by an explicit history request.
    History,
}

/// An owned incoming `PRIVMSG` or `NOTICE`.
///
/// Correlated copies of this client's sends are returned through [`crate::Sent`] and are not
/// emitted as messages. Applications that log everything must therefore log their own sends from
/// the send future's result.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Message {
    /// The user or server that sent the message.
    pub actor: Source,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// The IRC conversation to which the message belongs.
    pub conversation: Conversation,
    /// `STATUSMSG` prefixes removed from the wire address, in wire order.
    pub status: Option<Vec<char>>,
    /// Display text with IRC formatting removed.
    pub text: String,
    /// Text with IRC formatting retained and the CTCP wrapper removed from an `ACTION`.
    pub raw_text: String,
    /// The semantic message kind.
    pub kind: MessageKind,
    /// The server-assigned message identifier, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, or the local receipt time when none was supplied.
    pub time: SystemTime,
    /// The actor's authenticated account, when tagged.
    pub account: Option<AccountName>,
    /// The identifier referenced by a reply tag.
    pub reply_to: Option<MessageId>,
    /// Channel context attached to a private message.
    pub channel_context: Option<ChannelName>,
    /// Nickname of the client that submitted a relayed message.
    #[cfg(feature = "unstable")]
    pub relayed_by: Option<Nick>,
    /// Whether the message carried the `bot` tag.
    pub is_bot: bool,
    /// IRC operator name, when the operator tag supplied one.
    pub oper: Option<String>,
    /// How the message relates to the current session.
    pub origin: Origin,
    /// All message tags in wire order.
    pub tags: Tags,
}

struct MessageMetadata {
    actor: Source,
    authorship: Authorship,
    conversation: Conversation,
    status: Option<Vec<char>>,
    msgid: Option<MessageId>,
    time: SystemTime,
    account: Option<AccountName>,
    reply_to: Option<MessageId>,
    channel_context: Option<ChannelName>,
    #[cfg(feature = "unstable")]
    relayed_by: Option<Nick>,
    is_bot: bool,
    oper: Option<String>,
    origin: Origin,
    tags: Tags,
}

impl Ctcp {
    pub(crate) fn new(
        raw: ProtoLine,
        isupport: &Isupport,
        nick: &str,
        received_at: SystemTime,
    ) -> Result<Self, ProtoLine> {
        let own_source = raw.as_ref().source().is_some_and(|source| {
            matches!(source, papo_proto::Source::User { nick: source, .. }
                if isupport.casemapping().nick(source) == nick)
        });
        let Ok((status, conversation)) =
            papo_core::classify_conversation(&raw.as_ref(), nick, isupport)
        else {
            return Err(raw);
        };
        Self::from_line(
            raw,
            None,
            if own_source {
                Authorship::Own
            } else {
                Authorship::Other
            },
            conversation,
            status,
            isupport,
            received_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_line(
        raw: ProtoLine,
        batch_type: Option<&str>,
        authorship: Authorship,
        conversation: Conversation,
        status: Option<Vec<char>>,
        isupport: &Isupport,
        received_at: SystemTime,
    ) -> Result<Self, ProtoLine> {
        let message = raw.as_ref();
        let Some((command, params, reply)) = ctcp_parts(&message) else {
            return Err(raw);
        };
        let Some(metadata) = message_metadata(
            &message,
            batch_type,
            authorship,
            conversation,
            status,
            isupport,
            received_at,
        ) else {
            return Err(raw);
        };
        Ok(Self {
            actor: metadata.actor,
            authorship: metadata.authorship,
            conversation: metadata.conversation,
            status: metadata.status,
            command,
            params,
            reply,
            msgid: metadata.msgid,
            time: metadata.time,
            account: metadata.account,
            origin: metadata.origin,
            tags: metadata.tags,
        })
    }
}

fn ctcp_parts(message: &LineRef<'_>) -> Option<(String, Option<String>, bool)> {
    let reply = match () {
        () if message.command_is("PRIVMSG") => false,
        () if message.command_is("NOTICE") => true,
        () => return None,
    };
    let ctcp = ProtoCtcp::parse(message.param(1)?)?;
    (reply || !ctcp.command.eq_ignore_ascii_case("ACTION")).then(|| {
        (
            ctcp.command.to_owned(),
            ctcp.params.map(str::to_owned),
            reply,
        )
    })
}

pub(crate) fn incoming_from_core(
    received: papo_core::ReceivedLine,
    isupport: &Isupport,
    received_at: SystemTime,
) -> Event {
    let conversation = received.conversation().cloned().filter(|_| {
        ctcp_parts(&received.as_ref()).is_some() && received.as_ref().source().is_some()
    });
    let Some(conversation) = conversation else {
        return Message::from_core(received, isupport, received_at)
            .map_or_else(Event::Other, |message| Event::Message(Box::new(message)));
    };
    let status = received.status().map(<[char]>::to_vec);
    let authorship = Authorship::from_core(received.authorship());
    let (raw, batch_type) = received.into_parts();
    Event::Ctcp(
        Ctcp::from_line(
            raw,
            batch_type.as_deref(),
            authorship,
            conversation,
            status,
            isupport,
            received_at,
        )
        .unwrap_or_else(|_| unreachable!("validated CTCP line changed while moving ownership")),
    )
}

pub(crate) fn incoming(
    line: ProtoLine,
    isupport: &Isupport,
    nick: &str,
    received_at: SystemTime,
) -> Event {
    match Ctcp::new(line, isupport, nick, received_at) {
        Ok(ctcp) => Event::Ctcp(ctcp),
        Err(line) => Message::new(line, isupport, nick, received_at)
            .map_or_else(Event::Other, |message| Event::Message(Box::new(message))),
    }
}

impl Message {
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
        let authorship = if own_source {
            Authorship::Own
        } else {
            Authorship::Other
        };
        let Ok((status, conversation)) =
            papo_core::classify_conversation(&raw.as_ref(), nick, isupport)
        else {
            return Err(raw);
        };
        Self::new_with_context(
            raw,
            None,
            authorship,
            conversation,
            status,
            isupport,
            received_at,
        )
    }

    pub(crate) fn from_core(
        received: papo_core::ReceivedLine,
        isupport: &Isupport,
        received_at: SystemTime,
    ) -> Result<Self, ProtoLine> {
        let Some(conversation) = received.conversation().cloned() else {
            return Err(received.into_line());
        };
        let status = received.status().map(<[char]>::to_vec);
        let authorship = Authorship::from_core(received.authorship());
        let (raw, batch_type) = received.into_parts();
        Self::new_with_context(
            raw,
            batch_type.as_deref(),
            authorship,
            conversation,
            status,
            isupport,
            received_at,
        )
    }

    pub(crate) fn from_history(
        message: ProtoLine,
        authorship: Authorship,
        client: &Client,
        received_at: SystemTime,
    ) -> Result<Self, ProtoLine> {
        Self::from_historical(message, client, received_at, Origin::History, authorship)
    }

    pub(crate) fn from_replay(
        message: ProtoLine,
        authorship: Authorship,
        client: &Client,
        received_at: SystemTime,
    ) -> Result<Self, ProtoLine> {
        Self::from_historical(message, client, received_at, Origin::Replay, authorship)
    }

    fn from_historical(
        message: ProtoLine,
        client: &Client,
        received_at: SystemTime,
        origin: Origin,
        authorship: Authorship,
    ) -> Result<Self, ProtoLine> {
        let snapshot = client.state();
        let Ok((status, conversation)) = papo_core::classify_conversation(
            &message.as_ref(),
            snapshot.nick(),
            snapshot.isupport().inner(),
        ) else {
            return Err(message);
        };
        let mut event = Self::new_with_context(
            message,
            None,
            authorship,
            conversation,
            status,
            snapshot.isupport().inner(),
            received_at,
        )?;
        event.origin = origin;
        Ok(event)
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_context(
        raw: ProtoLine,
        batch_type: Option<&str>,
        authorship: Authorship,
        conversation: Conversation,
        status: Option<Vec<char>>,
        isupport: &Isupport,
        received_at: SystemTime,
    ) -> Result<Self, ProtoLine> {
        let message = raw.as_ref();
        let wallops = message.command_is("WALLOPS");
        let plain = if message.command_is("PRIVMSG") {
            MessageKind::Privmsg
        } else if message.command_is("NOTICE") || wallops {
            MessageKind::Notice
        } else {
            return Err(raw);
        };
        let Some(raw_text) = message.param(usize::from(!wallops)) else {
            return Err(raw);
        };
        let (kind, displayed) = match (!wallops).then(|| ProtoCtcp::parse(raw_text)).flatten() {
            Some(ctcp)
                if plain == MessageKind::Privmsg && ctcp.command.eq_ignore_ascii_case("ACTION") =>
            {
                (MessageKind::Action, ctcp.params.unwrap_or(""))
            }
            Some(_) => return Err(raw),
            None => (plain, raw_text),
        };
        let text = strip_formatting(displayed).into_owned();
        let raw_text = displayed.to_owned();
        let Some(metadata) = message_metadata(
            &message,
            batch_type,
            authorship,
            conversation,
            status,
            isupport,
            received_at,
        ) else {
            return Err(raw);
        };
        Ok(Self {
            actor: metadata.actor,
            authorship: metadata.authorship,
            conversation: metadata.conversation,
            status: metadata.status,
            text,
            raw_text,
            kind,
            msgid: metadata.msgid,
            time: metadata.time,
            account: metadata.account,
            reply_to: metadata.reply_to,
            channel_context: metadata.channel_context,
            #[cfg(feature = "unstable")]
            relayed_by: metadata.relayed_by,
            is_bot: metadata.is_bot,
            oper: metadata.oper,
            origin: metadata.origin,
            tags: metadata.tags,
        })
    }

    pub(crate) fn channel_context(&self) -> Option<&str> {
        self.channel_context.as_deref()
    }

    /// Returns the formatted spans parsed from [`Self::raw_text`].
    pub fn spans(&self) -> impl Iterator<Item = crate::Span<'_>> {
        papo_proto::Formatted::new(&self.raw_text)
    }

    /// Returns the plain-text IRC address that reaches the same recipients.
    #[must_use]
    pub fn return_address(&self) -> String {
        match &self.conversation {
            Conversation::Channel(channel) => self
                .status
                .iter()
                .flatten()
                .copied()
                .chain(channel.chars())
                .collect(),
            Conversation::Private(nick) => nick.to_string(),
            Conversation::Broadcast(mask) => mask.to_string(),
            Conversation::Server => self.actor.to_string(),
            _ => "*".to_owned(),
        }
    }

    /// Returns server time when valid, or the local receipt time.
    #[must_use]
    pub fn time(&self) -> SystemTime {
        self.time
    }

    /// Returns the message identifier referenced by a reply tag.
    #[must_use]
    pub const fn reply_to(&self) -> Option<&MessageId> {
        self.reply_to.as_ref()
    }

    /// Reports whether a bot may treat this as a newly received message.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.origin == Origin::Live
    }

    /// Reports whether this message's actor is this client's user.
    #[must_use]
    pub fn is_own(&self) -> bool {
        self.authorship == Authorship::Own
    }
}

#[allow(clippy::too_many_arguments)]
fn message_metadata(
    message: &LineRef<'_>,
    batch_type: Option<&str>,
    authorship: Authorship,
    conversation: Conversation,
    status: Option<Vec<char>>,
    isupport: &Isupport,
    received_at: SystemTime,
) -> Option<MessageMetadata> {
    let actor = message
        .source()
        .map(|source| source::from_proto(source, isupport.casemapping()))?;
    let tags = Tags::from_message(message);
    let channel_context = if matches!(conversation, Conversation::Private(_)) {
        tag_owned(message, &["+channel-context", "+draft/channel-context"])
            .map(|channel| isupport.casemapping().channel(channel))
    } else {
        None
    };
    let envelope = line_envelope(message, isupport.casemapping(), received_at);
    let reply_to = tag_owned(message, &["+reply", "+draft/reply"]).map(MessageId::from);
    #[cfg(feature = "unstable")]
    let relayed_by = tag_owned(message, &[crate::cap::RELAYMSG.as_str()])
        .map(|nick| isupport.casemapping().nick(nick));
    let oper_tag = message.tags().find(|(key, _)| *key == "draft/oper");
    let replay = batch_type.is_some_and(|kind| !matches!(kind, "netsplit" | "netjoin"));
    Some(MessageMetadata {
        actor,
        authorship,
        conversation,
        status,
        msgid: envelope.msgid,
        time: envelope.time,
        account: envelope.account,
        reply_to,
        channel_context,
        #[cfg(feature = "unstable")]
        relayed_by,
        is_bot: tags.contains("bot"),
        oper: oper_tag.and_then(|(_, value)| value.map(std::borrow::Cow::into_owned)),
        origin: if replay { Origin::Replay } else { Origin::Live },
        tags,
    })
}

pub(crate) struct EventEnvelope {
    pub(crate) msgid: Option<MessageId>,
    pub(crate) time: SystemTime,
    pub(crate) account: Option<AccountName>,
}

pub(crate) fn line_envelope(
    line: &LineRef<'_>,
    casemapping: Casemapping,
    received_at: SystemTime,
) -> EventEnvelope {
    EventEnvelope {
        msgid: tag_owned(line, &["msgid"]).map(MessageId::from),
        time: tag_owned(line, &["time", "znc.in/server-time-iso"])
            .and_then(|value| time::parse(&value))
            .unwrap_or(received_at),
        account: tag_owned(line, &["account"]).map(|account| casemapping.account(account)),
    }
}

pub(crate) fn joined_from_line(
    line: &LineRef<'_>,
    authorship: Authorship,
    casemapping: Casemapping,
    received_at: SystemTime,
) -> Option<Joined> {
    let join = papo_proto::Join::parse(line)?;
    let source @ papo_proto::Source::User { nick, .. } = line.source()? else {
        return None;
    };
    let nick = casemapping.nick(nick);
    let envelope = line_envelope(line, casemapping, received_at);
    let account = match (join.realname, join.account) {
        (None, _) => Account::Unknown,
        (Some(_), None) => Account::LoggedOut,
        (Some(_), Some(account)) => Account::LoggedIn(casemapping.account(account)),
    };
    Some(Joined {
        msgid: envelope.msgid,
        time: envelope.time,
        account,
        realname: join.realname.map(str::to_owned),
        channel: casemapping.channel(join.channel),
        actor: source::from_proto(source, casemapping),
        nick,
        authorship,
    })
}

fn core_envelope(
    envelope: &papo_core::LineEnvelope,
    casemapping: Casemapping,
    received_at: SystemTime,
) -> EventEnvelope {
    EventEnvelope {
        msgid: envelope.msgid().map(MessageId::from),
        time: envelope.time().unwrap_or(received_at),
        account: envelope
            .account()
            .map(|account| casemapping.account(account)),
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

/// An event emitted by an IRC client.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// The IRC connection is ready for use.
    Connected(Connected),
    /// A complete message of the day received while connecting.
    Motd(Motd),
    /// An incoming `PRIVMSG` or `NOTICE`.
    Message(Box<Message>),
    /// An incoming Client-to-Client Protocol query or reply.
    Ctcp(Ctcp),
    /// A line papo did not turn into a typed event.
    ///
    /// This includes unknown numerics, vendor commands, and new drafts. Match it only for display
    /// and logging; a future version may type it.
    Other(ProtoLine),
    /// A tag-only message with typed accessors for known client tags.
    TagMsg(TagMsg),
    /// A tag-only reaction or unreaction.
    Reacted(Reacted),
    /// A tag-only typing notification.
    TypingChanged(TypingChanged),
    /// A message was redacted from a conversation's history.
    Redacted(Redacted),
    /// An unsolicited `IRCv3` standard reply.
    StandardReply(StandardReply),
    /// Account creation completed and the connection is authenticated.
    AccountCreation(crate::AccountCreation),
    /// The server reported the account's persistence state.
    #[cfg(feature = "unstable")]
    PersistenceChanged(crate::PersistenceStatus),
    /// An upstream bouncer network was added, updated, or removed.
    #[cfg(feature = "unstable")]
    BouncerNetwork(BouncerNetworkChanged),
    /// A user or channel's cached metadata changed.
    MetadataChanged(MetadataChanged),
    /// A completed history or playback batch.
    History(Box<crate::HistoryPage>),
    /// A conversation's read marker advanced or was first reported.
    ReadMarkerChanged(ReadMarkerChanged),
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
    /// Optional behaviors available on an established connection changed.
    FeaturesChanged(FeaturesChanged),
    /// Server-defined name comparison rules changed between connections.
    CasemappingChanged(CasemappingChanged),
    /// A channel join configured or remembered by papo was refused.
    AutoJoinFailed {
        /// The channel papo tried to join.
        channel: ChannelName,
        /// The server's typed refusal.
        error: crate::ServerError,
    },
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
    (boxed Message, as_message, Message),
    (Ctcp, as_ctcp, Ctcp),
    (boxed History, as_history, crate::HistoryPage),
    (Connected, as_connected, Connected),
    (Motd, as_motd, Motd),
    (Disconnected, as_disconnected, Disconnected),
    (CapsChanged, as_caps_changed, CapsChanged),
    (FeaturesChanged, as_features_changed, FeaturesChanged),
    (CasemappingChanged, as_casemapping_changed, CasemappingChanged),
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
    (Netsplit, as_netsplit, Netsplit),
    (Netjoin, as_netjoin, Netjoin),
    (MetadataChanged, as_metadata_changed, MetadataChanged),
    (AccountCreation, as_account_creation, crate::AccountCreation),
    (Warning, as_warning, Warning),
    (Other, as_other, ProtoLine),
}

#[cfg(feature = "unstable")]
impl Event {
    /// Returns the payload when this is [`Event::PersistenceChanged`].
    #[must_use]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub const fn as_persistence_changed(&self) -> Option<&crate::PersistenceStatus> {
        if let Self::PersistenceChanged(payload) = self {
            Some(payload)
        } else {
            None
        }
    }

    /// Returns the payload when this is [`Event::BouncerNetwork`].
    #[must_use]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub const fn as_bouncer_network(&self) -> Option<&BouncerNetworkChanged> {
        if let Self::BouncerNetwork(payload) = self {
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
    Line(papo_core::ReceivedLine),
    Other(ProtoLine),
    Response(Response),
    RequestUntracked(RequestId),
    RequestTimedOut(RequestId),
    RequestAborted {
        id: RequestId,
        transmitted: bool,
    },
    TransmitAborted(papo_core::TransmitToken),
    StsUpgrade(u16),
    StsPolicy(StsAction),
    History {
        conversation: crate::Conversation,
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

// Keeping the exhaustive forwarding table together makes omissions visible when core grows.
#[allow(clippy::too_many_lines)]
pub(crate) fn from_core(
    event: papo_core::Event,
    casemapping: Casemapping,
    received_at: SystemTime,
) -> CoreEvent {
    CoreEvent::Forward(match event {
        papo_core::Event::Connected(welcome) => return CoreEvent::Connected(welcome),
        papo_core::Event::Motd { lines } => Event::Motd(Motd { lines }),
        papo_core::Event::Line(message) => {
            if message.as_ref().command_is("001") {
                return CoreEvent::Unknown;
            }
            return CoreEvent::Line(message);
        }
        papo_core::Event::MultilineMessage(message) => return CoreEvent::Other(message.message),
        papo_core::Event::TypingChanged(change) => Event::TypingChanged(TypingChanged {
            actor: source::from_prefix(change.actor, casemapping),
            authorship: Authorship::from_core(change.authorship),
            conversation: change.conversation,
            status: change.status,
            state: change.state,
            reason: change.reason,
        }),
        papo_core::Event::TagMessage(message) => {
            let envelope = core_envelope(&message.envelope, casemapping, received_at);
            let tag = TagMsg {
                actor: source::from_prefix(message.actor, casemapping),
                authorship: Authorship::from_core(message.authorship),
                conversation: message.conversation,
                status: message.status,
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
                    msgid: envelope.msgid,
                    time: envelope.time,
                    account: envelope.account,
                    actor: tag.actor,
                    authorship: tag.authorship,
                    conversation: tag.conversation,
                    status: tag.status,
                    message: msgid.into(),
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
                    actor: tag.actor,
                    authorship: tag.authorship,
                    conversation: tag.conversation,
                    status: tag.status,
                    state,
                    reason: TypingReason::Notified,
                })
            } else {
                Event::TagMsg(tag)
            }
        }
        papo_core::Event::Redacted(redacted) => {
            let envelope = core_envelope(&redacted.envelope, casemapping, received_at);
            Event::Redacted(Redacted {
                msgid: envelope.msgid,
                time: envelope.time,
                account: envelope.account,
                actor: source::from_prefix(redacted.actor, casemapping),
                authorship: Authorship::from_core(redacted.authorship),
                conversation: redacted.conversation,
                status: redacted.status,
                message: redacted.msgid.into(),
                reason: redacted.reason.filter(|reason| !reason.is_empty()),
            })
        }
        papo_core::Event::Response(response) => return CoreEvent::Response(response),
        papo_core::Event::RequestUntracked { id } => return CoreEvent::RequestUntracked(id),
        papo_core::Event::RequestTimedOut { id } => return CoreEvent::RequestTimedOut(id),
        papo_core::Event::RequestAborted { id, transmitted } => {
            return CoreEvent::RequestAborted { id, transmitted };
        }
        papo_core::Event::TransmitAborted { token } => return CoreEvent::TransmitAborted(token),
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
        papo_core::Event::AccountCreated { account, .. } => {
            Event::AccountCreation(crate::AccountCreation::Created { account })
        }
        papo_core::Event::VerificationRequired { account, message } => {
            Event::AccountCreation(crate::AccountCreation::VerificationRequired {
                account,
                message,
            })
        }
        #[cfg(feature = "unstable")]
        papo_core::Event::PersistenceStatus(status) => Event::PersistenceChanged(status),
        #[cfg(feature = "unstable")]
        papo_core::Event::BouncerNetwork { id, network } => {
            Event::BouncerNetwork(BouncerNetworkChanged { id, network })
        }
        papo_core::Event::MetadataChanged {
            subject,
            key,
            value,
            visibility,
        } => Event::MetadataChanged(MetadataChanged {
            subject: match subject {
                papo_core::Subject::Channel(channel) => crate::Subject::Channel(channel),
                papo_core::Subject::User(nick) => crate::Subject::User(nick),
                _ => return CoreEvent::Unknown,
            },
            key: key.into(),
            value,
            visibility,
        }),
        papo_core::Event::History {
            conversation,
            items,
            end,
        } => {
            return CoreEvent::History {
                conversation,
                items,
                end,
            };
        }
        papo_core::Event::ReadMarkerChanged(marker) => {
            Event::ReadMarkerChanged(ReadMarkerChanged {
                conversation: marker.conversation,
                time: marker.time,
            })
        }
        papo_core::Event::Netsplit {
            servers,
            quits,
            authorship,
        } => Event::Netsplit(Netsplit {
            servers: servers.into(),
            quits: quits
                .into_iter()
                .zip(authorship)
                .filter_map(|(message, authorship)| {
                    let message = message.as_ref();
                    let nick = match message.source()? {
                        papo_proto::Source::User { nick, .. } => nick.to_owned(),
                        papo_proto::Source::Server(_) => return None,
                    };
                    Some(Departed {
                        nick: casemapping.nick(nick),
                        authorship: Authorship::from_core(authorship),
                        reason: message.param(0).map(str::to_owned),
                    })
                })
                .collect(),
        }),
        papo_core::Event::Netjoin {
            servers,
            joins,
            authorship,
        } => Event::Netjoin(Netjoin {
            servers: servers.into(),
            joins: joins
                .into_iter()
                .zip(authorship)
                .filter_map(|(message, authorship)| {
                    joined_from_line(
                        &message.as_ref(),
                        Authorship::from_core(authorship),
                        casemapping,
                        received_at,
                    )
                })
                .collect(),
        }),
        papo_core::Event::BatchAbandoned {
            reference,
            batch_type,
            reason,
        } => Event::Warning(Warning::BatchAbandoned {
            id: reference,
            batch_type,
            reason,
        }),
        papo_core::Event::Malformed { line } => Event::Warning(Warning::Malformed { line }),
        papo_core::Event::Close { kind, reason } => return CoreEvent::Close { kind, reason },
        papo_core::Event::AuthenticationFailed { reason } => {
            Event::Warning(Warning::AuthenticationFailed { reason })
        }
        papo_core::Event::CapsChanged { added, removed } => {
            Event::CapsChanged(CapsChanged { added, removed })
        }
        papo_core::Event::FeaturesChanged { added, removed } => {
            Event::FeaturesChanged(FeaturesChanged { added, removed })
        }
        papo_core::Event::CasemappingChanged { old, new } => {
            Event::CasemappingChanged(CasemappingChanged { old, new })
        }
        papo_core::Event::StsUpgrade { port } => return CoreEvent::StsUpgrade(port),
        papo_core::Event::StsPolicy(action) => return CoreEvent::StsPolicy(action),
        papo_core::Event::Joined {
            envelope,
            actor,
            account,
            realname,
            channel,
            nick,
            authorship,
        } => {
            let envelope = core_envelope(&envelope, casemapping, received_at);
            let actor = source::from_prefix(actor, channel.casemapping());
            Event::Joined(Joined {
                msgid: envelope.msgid,
                time: envelope.time,
                account,
                realname,
                channel,
                actor,
                nick,
                authorship: Authorship::from_core(authorship),
            })
        }
        papo_core::Event::ChannelSynced { channel } => {
            Event::ChannelSynced(ChannelSynced { channel })
        }
        papo_core::Event::MembersRefreshed { channel } => {
            Event::MembersRefreshed(MembersRefreshed { channel })
        }
        papo_core::Event::Parted {
            envelope,
            channel,
            nick,
            reason,
            authorship,
        } => {
            let envelope = core_envelope(&envelope, casemapping, received_at);
            Event::Parted(Parted {
                msgid: envelope.msgid,
                time: envelope.time,
                account: envelope.account,
                channel,
                nick,
                reason,
                authorship: Authorship::from_core(authorship),
            })
        }
        papo_core::Event::Kicked {
            envelope,
            channel,
            nick,
            actor,
            authorship,
            subject_authorship,
            reason,
            ..
        } => {
            let envelope = core_envelope(&envelope, casemapping, received_at);
            Event::Kicked(Kicked {
                msgid: envelope.msgid,
                time: envelope.time,
                account: envelope.account,
                channel,
                nick,
                actor: source::from_name(&actor, casemapping),
                authorship: Authorship::from_core(authorship),
                subject_authorship: Authorship::from_core(subject_authorship),
                reason,
            })
        }
        papo_core::Event::Invited {
            channel,
            nick,
            actor,
            authorship,
            subject_authorship,
        } => Event::Invited(Invited {
            channel,
            nick,
            actor: source::from_prefix(actor, casemapping),
            authorship: Authorship::from_core(authorship),
            subject_authorship: Authorship::from_core(subject_authorship),
        }),
        papo_core::Event::Quit {
            envelope,
            nick,
            reason,
            channels,
            authorship,
        } => {
            let envelope = core_envelope(&envelope, casemapping, received_at);
            Event::Quit(Quit {
                msgid: envelope.msgid,
                time: envelope.time,
                account: envelope.account,
                nick,
                reason,
                channels,
                authorship: Authorship::from_core(authorship),
            })
        }
        papo_core::Event::NickChanged {
            envelope,
            old,
            new,
            authorship,
        } => {
            let envelope = core_envelope(&envelope, casemapping, received_at);
            Event::NickChanged(NickChanged {
                msgid: envelope.msgid,
                time: envelope.time,
                account: envelope.account,
                old,
                new,
                authorship: Authorship::from_core(authorship),
            })
        }
        papo_core::Event::ChannelRenamed {
            old,
            new,
            actor,
            authorship,
            reason,
            ..
        } => Event::ChannelRenamed(ChannelRenamed {
            old,
            new,
            actor: source::from_prefix(actor, casemapping),
            authorship: Authorship::from_core(authorship),
            reason,
        }),
        papo_core::Event::AccountChanged { nick, account } => {
            Event::AccountChanged(AccountChanged {
                nick,
                account: match account {
                    Account::LoggedIn(account) => Some(account),
                    _ => None,
                },
            })
        }
        papo_core::Event::AwayChanged { nick, away } => {
            Event::AwayChanged(AwayChanged { nick, away })
        }
        papo_core::Event::HostChanged {
            nick,
            username,
            host,
        } => Event::HostChanged(HostChanged {
            nick,
            username,
            host,
        }),
        papo_core::Event::RealnameChanged { nick, realname } => {
            Event::RealnameChanged(RealnameChanged { nick, realname })
        }
        papo_core::Event::ModeChanged {
            envelope,
            subject,
            actor,
            authorship,
            subject_authorship,
            changes,
            ..
        } => {
            let envelope = core_envelope(&envelope, casemapping, received_at);
            Event::ModeChanged(ModeChanged {
                msgid: envelope.msgid,
                time: envelope.time,
                account: envelope.account,
                subject: match subject {
                    papo_core::Subject::Channel(channel) => crate::Subject::Channel(channel),
                    papo_core::Subject::User(nick) => crate::Subject::User(nick),
                    _ => return CoreEvent::Unknown,
                },
                actor: source::from_name(&actor, casemapping),
                authorship: Authorship::from_core(authorship),
                subject_authorship: Authorship::from_core(subject_authorship),
                changes,
            })
        }
        papo_core::Event::TopicChanged {
            envelope,
            channel,
            topic,
            actor,
            authorship,
        } => {
            let envelope = core_envelope(&envelope, casemapping, received_at);
            Event::TopicChanged(TopicChanged {
                msgid: envelope.msgid,
                time: envelope.time,
                account: envelope.account,
                channel,
                topic,
                actor: source::from_name(&actor, casemapping),
                authorship: Authorship::from_core(authorship),
            })
        }
        papo_core::Event::StateLimitReached { limit } => {
            Event::Warning(Warning::StateLimitReached { kind: limit })
        }
        papo_core::Event::MonitorOnline {
            nick,
            username,
            host,
        } => Event::MonitorOnline(MonitorOnline {
            nick,
            username,
            host,
        }),
        papo_core::Event::MonitorOffline { nick } => Event::MonitorOffline(MonitorOffline { nick }),
        papo_core::Event::MonitorListFull { limit, nicks } => {
            Event::Warning(Warning::MonitorListFull {
                maximum: limit,
                nicks,
            })
        }
        papo_core::Event::MonitorUnsupported => Event::Warning(Warning::MonitorUnsupported),
        _ => return CoreEvent::Unknown,
    })
}

pub(crate) fn typed_mode_change(
    action: ModeAction,
    mode: char,
    arg: Option<&str>,
    kind: papo_proto::ModeKind,
    casemapping: Casemapping,
) -> ModeChange {
    let arg = arg.map(|arg| match kind {
        papo_proto::ModeKind::Prefix(_) => ModeArg::Nick(casemapping.nick(arg)),
        papo_proto::ModeKind::List => ModeArg::Mask(casemapping.mask(arg)),
        _ => ModeArg::Value(arg.to_owned()),
    });
    ModeChange { mode, action, arg }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::time::{Duration, Instant, UNIX_EPOCH};

    use super::{
        Authorship, Conversation, CoreEvent, Event, Message as ChatMessage, MessageId, MessageKind,
        MetadataChanged, ModeAction, ModeArg, Origin, Source, StandardReplyKind, Visibility,
        Warning, ctcp_parts, from_core as from_core_with_mapping, incoming,
    };
    use papo_core::{
        Account, Away, BatchAbandonReason, Config, Feature, Session,
        StandardReply as CoreStandardReply, StateLimit,
    };
    use papo_proto::{Casemapping, Isupport, Line, LineRef, Typing};

    fn from_core(event: papo_core::Event) -> CoreEvent {
        from_core_with_mapping(event, Casemapping::default(), UNIX_EPOCH)
    }

    fn event(
        message: Line,
        isupport: &Isupport,
        nick: &str,
        received_at: std::time::SystemTime,
    ) -> ChatMessage {
        ChatMessage::new(message, isupport, nick, received_at).unwrap()
    }

    fn chat(command: &str, text: &str) -> ChatMessage {
        event(
            Line::new(command, ["#papo", text]).with_source("emily!~emily@localhost"),
            &Isupport::default(),
            "papo",
            UNIX_EPOCH,
        )
    }

    #[test]
    fn classifies_chat_and_ctcp_messages() {
        assert_eq!(chat("PRIVMSG", "hello").kind, MessageKind::Privmsg);
        assert_eq!(chat("NOTICE", "hello").kind, MessageKind::Notice);
        assert_eq!(
            chat("PRIVMSG", "\x01ACTION waves\x01").kind,
            MessageKind::Action
        );
        assert_eq!(
            chat("PRIVMSG", "\x01ACTION tests a split message").kind,
            MessageKind::Action
        );
        let parse = |command, text| {
            incoming(
                Line::new(command, ["#papo", text]).with_source("emily!~emily@localhost"),
                &Isupport::default(),
                "papo",
                UNIX_EPOCH,
            )
        };
        let version_event = parse("PRIVMSG", "\x01VERSION\x01");
        let version = version_event.as_ctcp().unwrap();
        assert_eq!(version.command, "VERSION");
        assert_eq!(version.params, None);
        assert!(!version.reply);
        let reply_event = parse("NOTICE", "\x01VERSION papo-0.1\x01");
        let reply = reply_event.as_ctcp().unwrap();
        assert_eq!(reply.params.as_deref(), Some("papo-0.1"));
        assert!(reply.reply);
        assert!(
            ChatMessage::new(
                Line::new("FAIL", ["PRIVMSG", "\x01VERSION\x01"]),
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
    fn distinguishes_ctcp_queries_and_replies() {
        let query = Line::privmsg("#papo", "\x01VERSION\x01");
        let (_, _, reply) = ctcp_parts(&query.as_ref()).unwrap();
        assert!(!reply);

        let response = Line::new("NOTICE", ["#papo", "\x01VERSION papo-0.1\x01"]);
        let (_, _, reply) = ctcp_parts(&response.as_ref()).unwrap();
        assert!(reply);

        let unrelated = Line::new("FAIL", ["PRIVMSG", "\x01VERSION\x01"]);
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
        assert_eq!(
            channel.actor,
            Source::from_proto(
                papo_proto::Source::User {
                    nick: "emily",
                    username: Some("~emily"),
                    host: Some("localhost")
                },
                Casemapping::default(),
            )
        );

        let private = event(
            Line::privmsg("papo", "hello").with_source("emily!~emily@localhost"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        assert!(matches!(private.conversation, Conversation::Private(_)));
        assert!(matches!(&private.conversation, Conversation::Private(nick) if nick == "emily"));
        assert_eq!(private.return_address(), "emily");

        let server = event(
            Line::notice("papo", "Server maintenance begins soon").with_source("irc.example"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        assert_eq!(server.conversation, Conversation::Server);
        assert_eq!(server.return_address(), "irc.example");

        let broadcast = event(
            Line::notice("$*.example.org", "Maintenance begins soon").with_source("irc.example"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        assert!(
            matches!(&broadcast.conversation, Conversation::Broadcast(mask) if mask == "$*.example.org")
        );
        assert_eq!(broadcast.return_address(), "$*.example.org");
    }

    #[test]
    fn ignores_channel_context_on_channel_messages() {
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
        assert_eq!(private.channel_context.as_deref(), Some("#papo"));
    }

    #[test]
    fn separates_status_prefixes_from_the_channel_target() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CHANTYPES=#", "STATUSMSG=@+"]);
        let message = event(
            Line::privmsg("@#ops", "operators only")
                .with_source("emily!~emily@localhost")
                .with_tag("bot", ""),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );

        assert_eq!(message.status.as_deref(), Some(['@'].as_slice()));
        assert!(matches!(&message.conversation, Conversation::Channel(name) if name == "#ops"));
        assert_eq!(message.return_address(), "@#ops");
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
            tagged.time(),
            UNIX_EPOCH + Duration::new(1_770_091_506, 789_000_000)
        );
        assert!(matches!(&tagged.conversation, Conversation::Channel(name) if name == "#papo"));
        assert_eq!(tagged.msgid.as_deref(), Some("RfcBTzyxY120004"));
        assert_eq!(tagged.account.as_deref(), Some("emily"));
        assert!(tagged.is_bot);
        let reply: Option<&MessageId> = tagged.reply_to();
        assert_eq!(reply.unwrap(), "RfcBTzyxY120003");
        #[cfg(feature = "unstable")]
        assert_eq!(tagged.relayed_by.as_deref(), Some("bridge-bot"));
        assert!(!chat("PRIVMSG", "human message").is_bot);

        for time in [None, Some("not-a-time")] {
            let mut message =
                Line::privmsg("#papo", "fallback").with_source("emily!~emily@localhost");
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
    fn message_names_capture_the_active_casemapping() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CASEMAPPING=ascii", "NICKLEN=30"]);
        let message = event(
            Line::privmsg("papo", "hello")
                .with_source("Alice[bot]!alice@cloak.example")
                .with_tag("account", "Alice[bot]")
                .with_tag("+channel-context", "#Papo[dev]")
                .with_tag("draft/relaymsg", "Bridge[bot]"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );

        let Source::User { nick, .. } = &message.actor else {
            panic!("message should have a user source");
        };
        assert_eq!(nick.casemapping(), Casemapping::Ascii);
        assert_eq!(
            message.account.as_ref().unwrap().casemapping(),
            Casemapping::Ascii
        );
        assert_eq!(
            message.channel_context.as_ref().unwrap().casemapping(),
            Casemapping::Ascii
        );
        assert_ne!(nick, "alice{bot}");
        #[cfg(feature = "unstable")]
        assert_eq!(
            message.relayed_by.as_ref().unwrap().casemapping(),
            Casemapping::Ascii
        );
    }

    #[test]
    fn exposes_oper_tags_with_optional_names() {
        let tagged = |line| {
            event(
                LineRef::parse(line).unwrap().to_owned(),
                &Isupport::default(),
                "papo",
                UNIX_EPOCH,
            )
        };
        assert_eq!(
            tagged(
                "@draft/oper=network-admin :alice!a@localhost PRIVMSG #papo :Scheduled maintenance"
            )
            .oper
            .as_deref(),
            Some("network-admin")
        );
        let unnamed = tagged("@draft/oper :alice!a@localhost PRIVMSG #papo :Scheduled maintenance");
        assert!(unnamed.tags.contains("draft/oper"));
        assert_eq!(unnamed.oper, None);
        let ordinary = chat("PRIVMSG", "ordinary message");
        assert!(!ordinary.tags.contains("draft/oper"));
        assert_eq!(ordinary.oper, None);
    }

    #[test]
    fn classifies_own_and_other_live_messages() {
        let classify = |message| event(message, &Isupport::default(), "Papo[bot]", UNIX_EPOCH);
        let echo = classify(Line::privmsg("#papo", "sent").with_source("papo{bot}!p@localhost"));
        assert_eq!(echo.origin, Origin::Live);
        assert!(echo.is_own());
        assert!(echo.is_live());
        let tagged = classify(
            Line::privmsg("#papo", "earlier")
                .with_source("emily!e@localhost")
                .with_tag("batch", "history42"),
        );
        assert_eq!(tagged.origin, Origin::Live);
        assert!(tagged.is_live());
        let live = classify(Line::privmsg("#papo", "now").with_source("emily!e@localhost"));
        assert_eq!(live.origin, Origin::Live);
        assert!(!live.is_own());
        assert!(live.is_live());
    }
    fn forwarded(event: papo_core::Event) -> Event {
        let CoreEvent::Forward(event) = from_core(event) else {
            panic!("event was not forwarded")
        };
        event
    }

    #[test]
    fn maps_feature_changes() {
        let event = forwarded(papo_core::Event::FeaturesChanged {
            added: vec![Feature::Reactions, Feature::Typing],
            removed: vec![Feature::History],
        });
        let changed = event.as_features_changed().unwrap();
        assert_eq!(changed.added, [Feature::Reactions, Feature::Typing]);
        assert_eq!(changed.removed, [Feature::History]);
    }

    #[test]
    fn maps_casemapping_changes() {
        let event = forwarded(papo_core::Event::CasemappingChanged {
            old: Casemapping::Ascii,
            new: Casemapping::Rfc1459Strict,
        });
        let changed = event.as_casemapping_changed().unwrap();
        assert_eq!(changed.old, Casemapping::Ascii);
        assert_eq!(changed.new, Casemapping::Rfc1459Strict);
    }

    #[test]
    fn maps_invitation_source_details() {
        let event = forwarded(papo_core::Event::Invited {
            channel: Casemapping::default().channel("#papo"),
            nick: Casemapping::default().nick("papo"),
            actor: "emily!~emily@localhost".to_owned(),
            authorship: papo_core::Authorship::Other,
            subject_authorship: papo_core::Authorship::Own,
        });
        let invited = event.as_invited().unwrap();
        assert_eq!(invited.channel, "#papo");
        assert_eq!(invited.nick, "papo");
        assert!(matches!(
            &invited.actor,
            Source::User { nick, username: Some(username), host: Some(host), .. }
                if nick == "emily" && username == "~emily" && host == "localhost"
        ));
        assert_eq!(invited.subject_authorship, Authorship::Own);
    }

    #[test]
    fn forwarded_names_keep_the_core_casemapping() {
        let CoreEvent::Forward(Event::Joined(joined)) = from_core_with_mapping(
            papo_core::Event::Joined {
                envelope: papo_core::LineEnvelope::default(),
                channel: Casemapping::Ascii.channel("#Papo[dev]"),
                actor: "Alice[bot]!~a@cloak.example".to_owned(),
                account: Account::Unknown,
                realname: None,
                nick: Casemapping::Ascii.nick("Alice[bot]"),
                authorship: papo_core::Authorship::Other,
            },
            Casemapping::Rfc1459,
            UNIX_EPOCH,
        ) else {
            panic!("join was not forwarded");
        };

        assert_eq!(joined.channel.casemapping(), Casemapping::Ascii);
        let Source::User { nick: actor, .. } = &joined.actor else {
            panic!("join actor should be a user");
        };
        assert_eq!(actor.casemapping(), Casemapping::Ascii);
        assert_eq!(joined.nick.casemapping(), Casemapping::Ascii);
        assert_ne!(joined.channel, "#papo{dev}");
        assert_ne!(joined.nick, "alice{bot}");
    }

    #[test]
    fn queued_event_keeps_authorship_across_a_nick_change() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(now, b":papo!papo@localhost JOIN #papo");
        session.handle_line(now, b":papo!papo@localhost NICK papo-away");

        let CoreEvent::Forward(Event::Joined(joined)) = from_core_with_mapping(
            session.poll_event().unwrap(),
            Casemapping::Rfc1459,
            UNIX_EPOCH,
        ) else {
            panic!("expected queued join");
        };
        let CoreEvent::Forward(Event::NickChanged(changed)) = from_core_with_mapping(
            session.poll_event().unwrap(),
            Casemapping::Rfc1459,
            UNIX_EPOCH,
        ) else {
            panic!("expected queued nick change");
        };
        assert_eq!(joined.authorship, Authorship::Own);
        assert_eq!(changed.authorship, Authorship::Own);
    }

    fn mapped_message(event: papo_core::Event) -> ChatMessage {
        let CoreEvent::Line(message) = from_core(event) else {
            panic!("message was not mapped")
        };
        ChatMessage::from_core(message, &Isupport::default(), UNIX_EPOCH).unwrap()
    }

    fn batched_message(kind: &str, command: &str) -> super::Message {
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
        let id = session.request(now, Line::new("MODE", ["#papo"])).unwrap();
        session.handle_line(now, b"@label=0 :ergo.test ACK");
        assert!(
            matches!(from_core(session.poll_event().unwrap()), CoreEvent::Response(response) if response.id == id)
        );
    }

    #[test]
    fn maps_multiline_as_a_normal_message() {
        let multiline = papo_core::MultilineMessage {
            message: Line::privmsg("#papo", "first\nsecond"),
        };
        assert!(matches!(
            from_core(papo_core::Event::MultilineMessage(multiline)),
            CoreEvent::Other(message) if message.as_ref().param(1) == Some("first\nsecond")
        ));
    }

    #[test]
    fn maps_tag_messages_with_typed_accessors() {
        let tag = papo_core::TagMessage {
            envelope: papo_core::LineEnvelope::default(),
            actor: "alice!~a@cloak".into(),
            authorship: papo_core::Authorship::Other,
            conversation: papo_core::Conversation::Channel(Casemapping::default().channel("#papo")),
            status: None,
            tags: vec![
                ("+typing".into(), Some("paused".into())),
                ("+draft/react".into(), Some("👍".into())),
                ("+reply".into(), Some("RKi9Cw".into())),
            ],
        };
        let Event::Reacted(reaction) = forwarded(papo_core::Event::TagMessage(tag)) else {
            panic!("expected reaction")
        };
        assert!(matches!(reaction.actor, Source::User { ref nick, .. } if nick == "alice"));
        assert!(
            matches!(&reaction.conversation, Conversation::Channel(channel) if channel == "#papo")
        );
        assert_eq!(reaction.status, None);
        assert_eq!(reaction.reaction, "👍");
        assert_eq!(reaction.message, "RKi9Cw");
        assert!(!reaction.removed);
    }

    #[test]
    fn distinguishes_every_typing_tag_value() {
        let typing = |value: &str| {
            let tag = papo_core::TagMessage {
                envelope: papo_core::LineEnvelope::default(),
                actor: "alice!~a@cloak".into(),
                authorship: papo_core::Authorship::Other,
                conversation: papo_core::Conversation::Channel(
                    Casemapping::default().channel("#papo"),
                ),
                status: None,
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
            envelope: papo_core::LineEnvelope::default(),
            actor: "alice!~a@cloak".into(),
            authorship: papo_core::Authorship::Other,
            conversation: papo_core::Conversation::Channel(Casemapping::default().channel("#papo")),
            status: None,
            tags: vec![("+typing".into(), Some("active".into()))],
        };
        assert!(
            matches!(forwarded(papo_core::Event::TagMessage(tag)), Event::TypingChanged(typing) if typing.state == Typing::Active)
        );
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
                    && matches!(change.actor, Source::User { ref nick, .. } if nick == "alice")
        ));
    }

    #[test]
    fn maps_redactions() {
        let redacted = papo_core::Redacted {
            envelope: papo_core::LineEnvelope::default(),
            actor: "alice!~a@cloak".into(),
            authorship: papo_core::Authorship::Other,
            conversation: papo_core::Conversation::Channel(Casemapping::default().channel("#papo")),
            status: None,
            msgid: "RKi9Cw".into(),
            reason: Some(String::new()),
        };
        assert!(matches!(
            forwarded(papo_core::Event::Redacted(redacted.clone())),
            Event::Redacted(mapped)
                if matches!(mapped.actor, Source::User { ref nick, .. } if nick == "alice")
                    && mapped.conversation == redacted.conversation
                    && mapped.status == redacted.status
                    && mapped.message.as_ref() == redacted.msgid
                    && mapped.reason.is_none()
        ));
    }

    #[cfg(feature = "unstable")]
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
    fn maps_request_timed_out() {
        let now = Instant::now();
        let mut session = labelled_session(now, Config::default().request_timeout(Duration::ZERO));
        let id = session.request(now, Line::new("MODE", ["#papo"])).unwrap();
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
        let id = session.request(now, Line::new("MODE", ["#papo"])).unwrap();
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
            assert_eq!(mapped.command, reply.command);
            assert_eq!(mapped.code, reply.code);
            assert_eq!(mapped.context, reply.context);
            assert_eq!(mapped.description, reply.description);
        }
    }

    #[test]
    fn maps_account_creation_results() {
        assert!(matches!(
            forwarded(papo_core::Event::AccountCreated {
                account: Casemapping::default().account("alice"),
                logged_in: true,
            }),
            Event::AccountCreation(crate::AccountCreation::Created { account })
                if account == "alice"
        ));
        assert!(matches!(
            forwarded(papo_core::Event::VerificationRequired {
                account: Casemapping::default().account("alice"),
                message: "Verification code sent".into(),
            }),
            Event::AccountCreation(crate::AccountCreation::VerificationRequired {
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
                old: Casemapping::default().channel("#rust"),
                new: Casemapping::default().channel("#rust-lang"),
                actor: "alice!a@localhost".into(),
                authorship: papo_core::Authorship::Other,
                reason: Some("Project renamed".into()),
            }),
            Event::ChannelRenamed(renamed)
                if renamed.old == "#rust"
                    && renamed.new == "#rust-lang"
                    && renamed.actor.nick().is_some_and(|nick| nick == "alice")
                    && renamed.reason.as_deref() == Some("Project renamed")
        ));

        assert!(matches!(
            forwarded(papo_core::Event::ChannelRenamed {
                old: Casemapping::default().channel("#rust"),
                new: Casemapping::default().channel("#rust-lang"),
                actor: "ergo.test".into(),
                authorship: papo_core::Authorship::Other,
                reason: None,
            }),
            Event::ChannelRenamed(renamed)
                if matches!(renamed.actor, Source::Server { ref name, .. } if name == "ergo.test")
                    && renamed.reason.is_none()
        ));
    }

    #[cfg(feature = "unstable")]
    #[test]
    fn maps_bouncer_network_updates() {
        let event = forwarded(papo_core::Event::BouncerNetwork {
            id: papo_core::NetworkId::from("42"),
            network: None,
        });
        let changed = event.as_bouncer_network().unwrap();
        assert_eq!(changed.id, "42");
        assert!(changed.network.is_none());
    }

    #[test]
    fn maps_metadata_events() {
        assert!(matches!(
            forwarded(papo_core::Event::MetadataChanged {
                subject: papo_core::Subject::Channel(Casemapping::default().channel("#papo")),
                key: "pronouns".into(),
                value: Some("they/them".into()),
                visibility: Visibility::Restricted("members".into()),
            }),
            Event::MetadataChanged(changed)
                if matches!(changed.subject, crate::Subject::Channel(ref channel) if channel == "#papo")
                    && changed.key == "pronouns"
                    && changed.value.as_deref() == Some("they/them")
                    && matches!(changed.visibility, Visibility::Restricted(ref policy) if policy == "members")
        ));
        assert!(matches!(
            forwarded(papo_core::Event::MetadataChanged {
                subject: papo_core::Subject::User(Casemapping::default().nick("alice")),
                key: "avatar".into(),
                value: None,
                visibility: Visibility::Public,
            }),
            Event::MetadataChanged(MetadataChanged {
                visibility: Visibility::Public,
                ..
            })
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
            "znc.in/playback",
            "vendor/sync",
        ] {
            let replay = batched_message(kind, "PRIVMSG #papo :earlier");
            assert_eq!(replay.origin, Origin::Replay);
            assert!(!replay.is_live());
        }

        for kind in ["netsplit", "netjoin"] {
            let live = batched_message(kind, "NOTICE papo :live batch");
            assert_eq!(live.origin, Origin::Live);
            assert!(live.is_live());
        }
    }

    #[test]
    fn explicit_history_has_a_non_live_origin() {
        let mut history = chat("PRIVMSG", "earlier");
        history.origin = Origin::History;
        assert_eq!(history.origin, Origin::History);
        assert!(!history.is_live());
    }

    #[test]
    fn classifies_an_own_message_as_live() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("Papo[bot]"));
        session.handle_line(now, b":papo{bot}!~p@host PRIVMSG #papo :sent");
        let echo = mapped_message(session.poll_event().expect("expected echo"));
        assert_eq!(echo.origin, Origin::Live);
        assert!(echo.is_live());
    }

    #[test]
    fn maps_joined() {
        let event = forwarded(papo_core::Event::Joined {
            envelope: papo_core::LineEnvelope::default(),
            channel: Casemapping::default().channel("#papo"),
            nick: Casemapping::default().nick("alice"),
            actor: "alice!~a@host".to_owned(),
            account: Account::LoggedIn(Casemapping::default().account("alice")),
            realname: Some("Alice Liddell".to_owned()),
            authorship: papo_core::Authorship::Other,
        });
        let joined = event.as_joined().unwrap();
        assert_eq!(joined.channel, "#papo");
        assert_eq!(joined.actor.nick().map(AsRef::as_ref), Some("alice"));
        assert_eq!(joined.nick, "alice");
        assert_eq!(joined.realname.as_deref(), Some("Alice Liddell"));
        assert!(matches!(&joined.account, Account::LoggedIn(account) if account == "alice"));
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
            received_at,
        );
        let CoreEvent::Forward(Event::Joined(joined)) = mapped else {
            panic!("expected mapped join");
        };
        assert_eq!(joined.msgid.as_deref(), Some("RfcBTzyxY120012"));
        assert_eq!(joined.time, received_at);
        assert_eq!(joined.account, Account::Unknown);
    }

    #[test]
    fn maps_channel_synced() {
        assert!(matches!(
            forwarded(papo_core::Event::ChannelSynced {
                channel: Casemapping::default().channel("#papo")
            }),
            Event::ChannelSynced(synced) if synced.channel == "#papo"
        ));
    }

    #[test]
    fn maps_members_refreshed() {
        assert!(matches!(
            forwarded(papo_core::Event::MembersRefreshed {
                channel: Casemapping::default().channel("#papo")
            }),
            Event::MembersRefreshed(refreshed) if refreshed.channel == "#papo"
        ));
    }

    #[test]
    fn maps_parted() {
        assert!(matches!(
            forwarded(papo_core::Event::Parted {
                envelope: papo_core::LineEnvelope::default(),
                channel: Casemapping::default().channel("#papo"),
                nick: Casemapping::default().nick("emily"),
                reason: Some("Leaving for lunch".into()),
                authorship: papo_core::Authorship::Other,
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
                envelope: papo_core::LineEnvelope::default(),
                channel: Casemapping::default().channel("#papo"),
                nick: Casemapping::default().nick("emily"),
                actor: "alice".into(),
                authorship: papo_core::Authorship::Other,
                subject_authorship: papo_core::Authorship::Other,
                reason: Some("Flooding".into()),
            }),
            Event::Kicked(kicked)
                if kicked.channel == "#papo"
                    && kicked.nick == "emily"
                    && kicked.actor.nick().is_some_and(|nick| nick == "alice")
                    && kicked.reason.as_deref() == Some("Flooding")
        ));
    }

    #[test]
    fn maps_quit() {
        assert!(matches!(
            forwarded(papo_core::Event::Quit {
                envelope: papo_core::LineEnvelope::default(),
                nick: Casemapping::default().nick("emily"),
                reason: Some("Client exited".into()),
                channels: vec![
                    Casemapping::default().channel("#papo"),
                    Casemapping::default().channel("#rust"),
                ],
                authorship: papo_core::Authorship::Other,
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
                envelope: papo_core::LineEnvelope::default(),
                old: Casemapping::default().nick("emily"),
                new: Casemapping::default().nick("em"),
                authorship: papo_core::Authorship::Other,
            }),
            Event::NickChanged(changed) if changed.old == "emily" && changed.new == "em"
        ));
    }

    #[test]
    fn maps_account_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::AccountChanged {
                nick: Casemapping::default().nick("emily"),
                account: Account::LoggedIn(Casemapping::default().account("emily")),
            }),
            Event::AccountChanged(changed)
                if changed.nick == "emily" && changed.account.as_deref() == Some("emily")
        ));
    }

    #[test]
    fn maps_away_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::AwayChanged {
                nick: Casemapping::default().nick("emily"),
                away: Away::Away {
                    message: Some("Gone to lunch".into()),
                },
            }),
            Event::AwayChanged(changed)
                if changed.nick == "emily"
                    && changed.away == Away::Away {
                        message: Some("Gone to lunch".into())
                    }
        ));
    }

    #[test]
    fn maps_host_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::HostChanged {
                nick: Casemapping::default().nick("emily"),
                username: "~em".into(),
                host: "users/emily".into(),
            }),
            Event::HostChanged(changed)
                if changed.nick == "emily" && changed.username == "~em" && changed.host == "users/emily"
        ));
    }

    #[test]
    fn maps_realname_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::RealnameChanged {
                nick: Casemapping::default().nick("emily"),
                realname: "Emily Smith".into(),
            }),
            Event::RealnameChanged(changed)
                if changed.nick == "emily" && changed.realname == "Emily Smith"
        ));
    }

    #[test]
    fn maps_mode_changed() {
        let mode_changes = vec![papo_core::ModeChange {
            mode: 'o',
            action: papo_core::ModeAction::Add,
            arg: Some(papo_core::ModeArg::Nick(
                Casemapping::default().nick("emily"),
            )),
        }];
        assert!(matches!(
            forwarded(papo_core::Event::ModeChanged {
                envelope: papo_core::LineEnvelope::default(),
                subject: papo_core::Subject::Channel(Casemapping::default().channel("#papo")),
                actor: "alice".into(),
                authorship: papo_core::Authorship::Other,
                subject_authorship: papo_core::Authorship::Other,
                changes: mode_changes.clone(),
            }),
            Event::ModeChanged(mode)
                if matches!(mode.subject, crate::Subject::Channel(ref channel) if channel == "#papo")
                    && mode.actor.nick().is_some_and(|nick| nick == "alice")
                    && mode.changes.len() == 1
                    && mode.changes[0].action == ModeAction::Add
                    && mode.changes[0].mode == 'o'
                    && matches!(&mode.changes[0].arg, Some(ModeArg::Nick(nick)) if nick == "emily")
        ));
    }

    #[test]
    fn maps_topic_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::TopicChanged {
                envelope: papo_core::LineEnvelope::default(),
                channel: Casemapping::default().channel("#papo"),
                topic: Some("Rust 1.90 is out".into()),
                actor: "alice".into(),
                authorship: papo_core::Authorship::Other,
            }),
            Event::TopicChanged(changed)
                if changed.channel == "#papo"
                    && changed.topic.as_deref() == Some("Rust 1.90 is out")
                    && changed.actor.nick().is_some_and(|nick| nick == "alice")
        ));
    }

    #[test]
    fn maps_state_limit_reached() {
        for (core, public) in [
            (StateLimit::Channels, super::StateLimit::Channels),
            (StateLimit::Members, super::StateLimit::Members),
            (StateLimit::ListEntries, super::StateLimit::ListEntries),
            (
                StateLimit::MetadataTargets,
                super::StateLimit::MetadataTargets,
            ),
            (StateLimit::MetadataKeys, super::StateLimit::MetadataKeys),
            #[cfg(feature = "unstable")]
            (
                StateLimit::BouncerNetworks,
                super::StateLimit::BouncerNetworks,
            ),
        ] {
            assert!(matches!(
                forwarded(papo_core::Event::StateLimitReached { limit: core }),
                Event::Warning(Warning::StateLimitReached { kind }) if kind == public
            ));
        }
    }

    #[test]
    fn maps_monitor_online_with_and_without_userhost() {
        assert!(matches!(
            forwarded(papo_core::Event::MonitorOnline {
                nick: Casemapping::default().nick("alice"),
                username: Some("~alice".into()),
                host: Some("cloak.example".into()),
            }),
            Event::MonitorOnline(online)
                if online.nick == "alice" && online.username.as_deref() == Some("~alice")
                    && online.host.as_deref() == Some("cloak.example")
        ));
        assert!(matches!(
            forwarded(papo_core::Event::MonitorOnline {
                nick: Casemapping::default().nick("bob"),
                username: None,
                host: None,
            }),
            Event::MonitorOnline(online)
                if online.nick == "bob" && online.username.is_none() && online.host.is_none()
        ));
    }

    #[test]
    fn maps_monitor_offline() {
        assert!(matches!(
            forwarded(papo_core::Event::MonitorOffline {
                nick: Casemapping::default().nick("alice"),
            }),
            Event::MonitorOffline(offline) if offline.nick == "alice"
        ));
    }

    #[test]
    fn maps_monitor_list_full() {
        assert!(matches!(
            forwarded(papo_core::Event::MonitorListFull {
                limit: 100,
                nicks: vec![
                    Casemapping::default().nick("alice"),
                    Casemapping::default().nick("bob"),
                ],
            }),
            Event::Warning(Warning::MonitorListFull { maximum, nicks })
                if maximum == 100 && nicks == ["alice", "bob"]
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
        let message = Line::privmsg("#papo", "earlier");
        let conversation = Conversation::Channel(Casemapping::Rfc1459.channel("#papo"));
        let CoreEvent::History {
            conversation: mapped,
            items,
            end,
        } = from_core(papo_core::Event::History {
            conversation: conversation.clone(),
            items: vec![
                papo_core::HistoryItem::Message(papo_core::HistoryLine {
                    line: message.clone(),
                    context: false,
                    authorship: papo_core::Authorship::Other,
                    subject_authorship: papo_core::Authorship::Unknown,
                }),
                papo_core::HistoryItem::Other(papo_core::HistoryLine {
                    line: Line::new("WALLOPS", ["Network maintenance begins soon"]),
                    context: false,
                    authorship: papo_core::Authorship::Unknown,
                    subject_authorship: papo_core::Authorship::Unknown,
                }),
            ],
            end: true,
        })
        else {
            panic!("history was not classified")
        };
        assert_eq!(mapped, conversation);
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
            Conversation::Channel(Casemapping::Rfc1459.channel("#papo")),
            UNIX_EPOCH + Duration::from_secs(42),
        )];
        assert!(matches!(
            from_core(papo_core::Event::HistoryTargets(targets)),
            CoreEvent::Unknown
        ));
    }

    #[test]
    fn maps_read_markers() {
        let marker = papo_core::ReadMarker {
            conversation: papo_core::Conversation::Channel(
                papo_proto::Casemapping::Rfc1459.channel("#papo"),
            ),
            time: Some(UNIX_EPOCH + Duration::from_secs(42)),
        };
        assert!(matches!(
            forwarded(papo_core::Event::ReadMarkerChanged(marker.clone())),
            Event::ReadMarkerChanged(mapped)
                if mapped.conversation == marker.conversation && mapped.time == marker.time
        ));
    }

    #[test]
    fn maps_netsplit() {
        let quit =
            Line::new("QUIT", ["irc.example irc.remote"]).with_source("alice!~a@remote.example");
        let Event::Netsplit(netsplit) = forwarded(papo_core::Event::Netsplit {
            servers: ["irc.example".into(), "irc.remote".into()],
            quits: vec![quit],
            authorship: vec![papo_core::Authorship::Other],
        }) else {
            panic!("netsplit was not forwarded")
        };
        assert_eq!(netsplit.servers.first, "irc.example");
        assert_eq!(netsplit.servers.second, "irc.remote");
        assert_eq!(netsplit.quits.len(), 1);
        assert_eq!(netsplit.quits[0].nick, "alice");
        assert_eq!(
            netsplit.quits[0].reason.as_deref(),
            Some("irc.example irc.remote")
        );
    }

    #[test]
    fn maps_netjoin() {
        let join = Line::new("JOIN", ["#papo", "*", "Bob"]).with_source("bob!~b@remote.example");
        let Event::Netjoin(netjoin) = forwarded(papo_core::Event::Netjoin {
            servers: ["irc.example".into(), "irc.remote".into()],
            joins: vec![join],
            authorship: vec![papo_core::Authorship::Other],
        }) else {
            panic!("netjoin was not forwarded")
        };
        assert_eq!(netjoin.servers.first, "irc.example");
        assert_eq!(netjoin.servers.second, "irc.remote");
        assert_eq!(netjoin.joins.len(), 1);
        assert_eq!(netjoin.joins[0].channel, "#papo");
        assert_eq!(
            netjoin.joins[0].actor.nick().map(AsRef::as_ref),
            Some("bob")
        );
        assert_eq!(netjoin.joins[0].nick, "bob");
        assert_eq!(netjoin.joins[0].account, Account::LoggedOut);
        assert_eq!(netjoin.joins[0].realname.as_deref(), Some("Bob"));
    }

    #[test]
    fn identifies_self_membership_and_nick_events_with_server_casemapping() {
        let mapping = Casemapping::Rfc1459;
        let actor = Source::from_proto(papo_proto::Source::Server("ergo.test"), mapping);
        let own_actor = Source::from_proto(
            papo_proto::Source::User {
                nick: "Papo[Bot]",
                username: None,
                host: None,
            },
            mapping,
        );
        let events = [
            Event::Joined(super::Joined {
                msgid: None,
                time: UNIX_EPOCH,
                account: Account::Unknown,
                realname: None,
                channel: mapping.channel("#papo"),
                actor: own_actor.clone(),
                nick: mapping.nick("Papo[Bot]"),
                authorship: Authorship::Own,
            }),
            Event::Parted(super::Parted {
                msgid: None,
                time: UNIX_EPOCH,
                account: None,
                channel: mapping.channel("#papo"),
                nick: mapping.nick("Papo[Bot]"),
                reason: None,
                authorship: Authorship::Own,
            }),
            Event::Kicked(super::Kicked {
                msgid: None,
                time: UNIX_EPOCH,
                account: None,
                channel: mapping.channel("#papo"),
                nick: mapping.nick("Papo[Bot]"),
                actor: actor.clone(),
                authorship: Authorship::Other,
                subject_authorship: Authorship::Own,
                reason: None,
            }),
            Event::Invited(super::Invited {
                channel: mapping.channel("#papo"),
                nick: mapping.nick("Papo[Bot]"),
                actor,
                authorship: Authorship::Other,
                subject_authorship: Authorship::Own,
            }),
            Event::NickChanged(super::NickChanged {
                msgid: None,
                time: UNIX_EPOCH,
                account: None,
                old: mapping.nick("Papo[Bot]"),
                new: mapping.nick("papo-away"),
                authorship: Authorship::Own,
            }),
            Event::Netjoin(super::Netjoin {
                servers: super::SplitServers {
                    first: "irc.example".to_owned(),
                    second: "irc.remote".to_owned(),
                },
                joins: vec![super::Joined {
                    msgid: None,
                    time: UNIX_EPOCH,
                    account: Account::Unknown,
                    realname: None,
                    channel: mapping.channel("#papo"),
                    actor: own_actor,
                    nick: mapping.nick("Papo[Bot]"),
                    authorship: Authorship::Own,
                }],
            }),
        ];

        assert_eq!(events[0].as_joined().unwrap().authorship, Authorship::Own);
        assert_eq!(events[1].as_parted().unwrap().authorship, Authorship::Own);
        assert_eq!(
            events[2].as_kicked().unwrap().subject_authorship,
            Authorship::Own
        );
        assert_eq!(
            events[3].as_invited().unwrap().subject_authorship,
            Authorship::Own
        );
        assert_eq!(
            events[4].as_nick_changed().unwrap().authorship,
            Authorship::Own
        );
        assert_eq!(
            events[5].as_netjoin().unwrap().joins[0].authorship,
            Authorship::Own
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
            }) = forwarded(papo_core::Event::BatchAbandoned {
                reference: "history42".into(),
                batch_type: "chathistory".into(),
                reason: core,
            })
            else {
                panic!("abandonment was not forwarded")
            };
            assert_eq!(id, "history42");
            assert_eq!(batch_type, "chathistory");
            assert_eq!(reason, public);
        }
    }
}
