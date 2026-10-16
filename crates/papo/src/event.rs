use papo_core::{
    Account, BatchAbandonReason, Capabilities, OwnedModeChange, RequestId, Response, StandardReply,
    StateLimit, Welcome, sts::StsAction,
};
use std::{
    fmt,
    time::{Duration, SystemTime},
};

use papo_proto::{
    Ctcp, Isupport, Message, MessageRef, Reaction, Source, Typing, strip_formatting, time,
};

use crate::{Client, Error};

/// The semantic kind of an incoming chat message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MessageKind<'a> {
    /// A regular `PRIVMSG`.
    Privmsg,
    /// A regular `NOTICE`.
    Notice,
    /// A CTCP `ACTION`.
    Action,
    /// Another CTCP command.
    Ctcp(&'a str),
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
    text: String,
    kind: StoredKind,
    isupport: Isupport,
    msgid: Option<String>,
    time: SystemTime,
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

#[derive(Clone, Debug, Eq, PartialEq)]
enum StoredKind {
    Privmsg,
    Notice,
    Action,
    Ctcp(String),
}

impl MessageEvent {
    #[cfg(feature = "bot")]
    pub(crate) const fn client(&self) -> &Client {
        &self.client
    }

    #[cfg(feature = "bot")]
    pub(crate) fn client_nick(&self) -> String {
        self.client.nick()
    }

    #[cfg(feature = "bot")]
    pub(crate) fn casemapping(&self) -> papo_proto::Casemapping {
        self.isupport.casemapping()
    }

    #[cfg(feature = "bot")]
    pub(crate) fn resolved_account(&self) -> Option<String> {
        if let Some(account) = &self.account {
            return Some(account.clone());
        }
        let Source::User { nick, .. } = self.source()? else {
            return None;
        };
        self.client
            .state(|state| match state.user(nick)?.account() {
                Account::LoggedIn(account) => Some(account.clone()),
                _ => None,
            })
    }

    pub(crate) fn new(
        raw: Message,
        isupport: &Isupport,
        nick: &str,
        client: Client,
        caps: &Capabilities,
        received_at: SystemTime,
    ) -> Result<Self, Message> {
        let echo = raw.as_ref().source().is_some_and(|source| {
            let Source::User { nick: source, .. } = source else {
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
        let snapshot = client.snapshot();
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
        let Some(raw_text) = message.param(0).and_then(|_| message.param(1)) else {
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
            Some(ctcp) if ctcp.command.eq_ignore_ascii_case("ACTION") => {
                (StoredKind::Action, ctcp.params.unwrap_or(""))
            }
            Some(ctcp) => (StoredKind::Ctcp(ctcp.command.to_owned()), raw_text),
            None => (plain, raw_text),
        };
        let text = strip_formatting(displayed).into_owned();
        let msgid = tag_owned(&message, &["msgid"]);
        let time = tag_owned(&message, &["time", "znc.in/server-time-iso"])
            .and_then(|value| time::parse(&value))
            .unwrap_or(received_at);
        let account = tag_owned(&message, &["account"]);
        let bot = message.tags().any(|(key, _)| key == "bot");
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
            text,
            kind,
            isupport: isupport.clone(),
            msgid,
            time,
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

    #[cfg(feature = "bot")]
    pub(crate) fn sender_nick(&self) -> Option<&str> {
        match self.source()? {
            Source::User { nick, .. } => Some(nick),
            Source::Server(_) => None,
        }
    }

    #[cfg(feature = "bot")]
    pub(crate) async fn private_notice(&self, text: &str) -> Result<(), Error> {
        let Some(target) = self.sender_nick() else {
            return Ok(());
        };
        let mut notice = Message::notice(target, text);
        if self.caps.contains_key("message-tags") {
            let denied = self.isupport.clienttagdeny();
            if let Some(msgid) = self.msgid()
                && let Some(tag) = allowed_alias(&denied, "+reply", "+draft/reply")
            {
                notice = notice.with_tag(tag, msgid);
            }
            let context = if self.is_private() {
                tag_owned(
                    &self.as_ref(),
                    &["+channel-context", "+draft/channel-context"],
                )
            } else {
                Some(self.target().to_owned())
            };
            if let Some(context) = context
                && let Some(tag) =
                    allowed_alias(&denied, "+channel-context", "+draft/channel-context")
            {
                notice = notice.with_tag(tag, context);
            }
        }
        self.client.send(notice).await
    }

    /// Returns the semantic kind of this message.
    #[must_use]
    pub fn kind(&self) -> MessageKind<'_> {
        match &self.kind {
            StoredKind::Privmsg => MessageKind::Privmsg,
            StoredKind::Notice => MessageKind::Notice,
            StoredKind::Action => MessageKind::Action,
            StoredKind::Ctcp(command) => MessageKind::Ctcp(command),
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
        self.raw.as_ref().param(1).unwrap_or("")
    }

    /// Returns the parsed source, when the server supplied one.
    #[must_use]
    pub fn source(&self) -> Option<Source<'_>> {
        self.raw.as_ref().source()
    }

    /// Returns the message target.
    #[must_use]
    pub fn target(&self) -> &str {
        self.raw.as_ref().param(0).unwrap_or("")
    }

    /// Reports whether the target is not a channel under the captured `CHANTYPES` value.
    #[must_use]
    pub fn is_private(&self) -> bool {
        !self.isupport.is_channel(self.target())
    }

    /// Returns the channel target, or the sender for a private message.
    #[must_use]
    pub fn reply_target(&self) -> &str {
        if self.is_private()
            && let Some(Source::User { nick, .. }) = self.source()
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

    /// Returns server time when valid, or the local receipt time.
    #[must_use]
    pub fn time(&self) -> SystemTime {
        self.time
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

    /// Sends a private message to [`Self::reply_target`].
    ///
    /// # Errors
    ///
    /// Returns an error if the message is invalid or the driver has stopped.
    pub async fn reply(&self, text: &str) -> Result<crate::Sent, Error> {
        let mut reply = Message::privmsg(self.reply_target(), text);
        if self.caps.contains_key("message-tags") {
            let denied = self.isupport.clienttagdeny();
            if let Some(msgid) = self.msgid()
                && let Some(tag) = allowed_alias(&denied, "+reply", "+draft/reply")
            {
                reply = reply.with_tag(tag, msgid);
            }
            if self.is_private()
                && let Some(context) = tag_owned(
                    &self.as_ref(),
                    &["+channel-context", "+draft/channel-context"],
                )
                && let Some(tag) =
                    allowed_alias(&denied, "+channel-context", "+draft/channel-context")
            {
                reply = reply.with_tag(tag, context);
            }
        }
        let tags = reply
            .as_ref()
            .tags()
            .filter(|(key, _)| key.starts_with('+'))
            .map(|(key, value)| (key.to_owned(), value.unwrap_or_default().into_owned()))
            .collect();
        self.client
            .send_text(
                papo_core::TextKind::Privmsg,
                self.reply_target(),
                text,
                tags,
            )
            .await
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

/// A tag-only message. Known tags have typed accessors; unknown tags remain available.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagMsg {
    source: Option<String>,
    target: String,
    tags: Vec<(String, Option<String>)>,
}

impl TagMsg {
    /// Returns the source prefix without its leading colon.
    #[must_use]
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    /// Returns the message target.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Returns all tags in wire order.
    #[must_use]
    pub fn tags(&self) -> &[(String, Option<String>)] {
        &self.tags
    }

    /// Returns a recognized typing state.
    #[must_use]
    pub fn typing(&self) -> Option<Typing> {
        match self.tag("+typing")? {
            "active" => Some(Typing::Active),
            "paused" => Some(Typing::Paused),
            "done" => Some(Typing::Done),
            _ => None,
        }
    }

    /// Returns a reaction linked to a message identifier.
    #[must_use]
    pub fn reaction(&self) -> Option<Reaction> {
        let react = self.tag("+draft/react").map(str::to_owned);
        let unreact = self.tag("+draft/unreact").map(str::to_owned);
        if react.is_none() && unreact.is_none() {
            return None;
        }
        Some(Reaction {
            react,
            unreact,
            reply_to: self
                .tag("+reply")
                .or_else(|| self.tag("+draft/reply"))?
                .to_owned(),
        })
    }

    fn tag(&self, key: &str) -> Option<&str> {
        self.tags
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == key)
            .and_then(|(_, value)| value.as_deref())
    }
}

/// An event emitted by an IRC client.
#[derive(Clone, Debug)]
#[non_exhaustive]
// Message events carry bounded connection snapshots used by their accessors and reply helper.
#[allow(clippy::large_enum_variant)]
pub enum Event {
    /// Connection registration completed.
    Registered(Welcome),
    /// A replacement connection completed registration.
    Reconnected(Welcome),
    /// An incoming `PRIVMSG` or `NOTICE`.
    Message(MessageEvent),
    /// A parsed message without a typed event variant.
    Raw(Message),
    /// A tag-only message with typed accessors for known client tags.
    TagMsg(TagMsg),
    /// A tag-only reaction or unreaction.
    Reaction(TagMsg),
    /// A tag-only typing notification.
    Typing(TagMsg),
    /// A message was redacted from a target's history.
    Redacted(papo_core::Redacted),
    /// A labelled request received its complete logical response.
    Response(Response),
    /// A request was sent without protocol-level response correlation.
    RequestUntracked {
        /// Identifier assigned to the request.
        id: RequestId,
    },
    /// A labelled request did not receive a response before its deadline.
    RequestTimedOut {
        /// Identifier assigned to the request.
        id: RequestId,
    },
    /// A labelled request was pending when the connection ended.
    RequestAborted {
        /// Identifier assigned to the request.
        id: RequestId,
    },
    /// An unsolicited `IRCv3` standard reply.
    StandardReply(StandardReply),
    /// Account creation completed and the connection is authenticated.
    AccountRegistered {
        /// The registered account name.
        account: String,
        /// Whether the server authenticated the connection to the account.
        logged_in: bool,
    },
    /// Account creation requires a verification step.
    VerificationRequired {
        /// The account awaiting verification.
        account: String,
        /// Human-readable instructions from the server.
        message: String,
    },
    /// The server reported the account's persistence state.
    PersistenceStatus {
        /// The configured persistence preference.
        setting: papo_core::PersistenceSetting,
        /// Whether persistence is currently effective.
        effective: bool,
    },
    /// A target's cached metadata changed.
    MetadataChanged {
        /// Nickname or channel reported by the server.
        target: String,
        /// Metadata key.
        key: String,
        /// New server-returned value, or `None` when cleared.
        value: Option<String>,
        /// Visibility policy reported by the server.
        visibility: String,
    },
    /// The server deferred metadata synchronization for a target.
    MetadataSyncLater {
        /// Nickname or channel whose synchronization was deferred.
        target: String,
        /// Suggested delay before retrying, when supplied.
        retry_after: Option<Duration>,
    },
    /// A completed history or playback batch.
    History {
        /// The history target named by the batch.
        target: String,
        /// Messages in server delivery order.
        messages: Vec<MessageEvent>,
        /// Whether the server reported that no further page exists.
        end: bool,
    },
    /// Targets that received messages during a requested time range.
    HistoryTargets(Vec<(String, SystemTime)>),
    /// A target's read marker advanced or was first reported.
    ReadMarker(papo_core::ReadMarker),
    /// Users departed because connectivity between two servers was lost.
    ///
    /// This event is emitted for an empty batch. Non-`QUIT` lines are delivered as
    /// separate [`Event::Message`] values.
    Netsplit {
        /// The servers on either side of the lost link.
        servers: [String; 2],
        /// The grouped `QUIT` messages.
        quits: Vec<Message>,
    },
    /// Users returned after connectivity between two servers was restored.
    ///
    /// This event is emitted for an empty batch. Non-`JOIN` lines are delivered as
    /// separate [`Event::Message`] values.
    Netjoin {
        /// The servers on either side of the restored link.
        servers: [String; 2],
        /// The grouped `JOIN` messages.
        joins: Vec<Message>,
    },
    /// A server batch exceeded a retention bound or deadline.
    BatchAbandoned {
        /// The opaque batch reference.
        reference: String,
        /// The batch type from its opening line.
        batch_type: String,
        /// The bound that was reached.
        reason: BatchAbandonReason,
    },
    /// A line from the server could not be decoded or parsed.
    Malformed {
        /// The original bytes, excluding the line ending.
        line: Vec<u8>,
    },
    /// Configured authentication failed and registration is continuing.
    AuthenticationFailed {
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
    },
    /// The capabilities enabled on an established connection changed.
    CapsChanged {
        /// Newly enabled capabilities and updated advertised values.
        added: Capabilities,
        /// Capabilities removed by the server.
        removed: Capabilities,
    },
    /// A plaintext connection must be replaced with TLS on `port`.
    StsUpgrade {
        /// The advertised TLS port.
        port: u16,
    },
    /// An STS persistence action for the client to store.
    StsPolicy(StsAction),
    /// A user joined a channel shared with the client.
    Joined {
        /// The joined channel.
        channel: String,
        /// The joining user's nickname.
        nick: String,
    },
    /// Initial channel membership synchronization completed.
    ChannelSynced {
        /// The synchronized channel.
        channel: String,
    },
    /// A user left a shared channel with `PART`.
    Parted {
        /// The departed channel.
        channel: String,
        /// The departing user's nickname.
        nick: String,
        /// The optional part reason.
        reason: Option<String>,
    },
    /// A user was removed from a shared channel with `KICK`.
    Kicked {
        /// The affected channel.
        channel: String,
        /// The removed user's nickname.
        nick: String,
        /// The user or server that issued the kick.
        by: String,
        /// The optional kick reason.
        reason: Option<String>,
    },
    /// A user sharing channels with the client disconnected.
    Quit {
        /// The disconnected user's nickname.
        nick: String,
        /// The optional quit reason.
        reason: Option<String>,
        /// Shared channels captured before the user was removed.
        channels: Vec<String>,
    },
    /// A user changed nickname.
    NickChanged {
        /// The previous nickname.
        old: String,
        /// The new nickname.
        new: String,
    },
    /// A joined channel was renamed.
    ChannelRenamed {
        /// The previous channel name.
        old: String,
        /// The new channel name.
        new: String,
        /// The server-supplied reason, which may be empty.
        reason: String,
    },
    /// A user's account state changed.
    AccountChanged {
        /// The affected user's nickname.
        nick: String,
        /// The user's new account state.
        account: Account,
    },
    /// A user's away state changed.
    AwayChanged {
        /// The affected user's nickname.
        nick: String,
        /// The away message, or `None` when the user returned.
        message: Option<String>,
    },
    /// A user's username or hostname changed.
    HostChanged {
        /// The affected user's nickname.
        nick: String,
        /// The user's new username.
        user: String,
        /// The user's new hostname.
        host: String,
    },
    /// A user's real name changed.
    RealnameChanged {
        /// The affected user's nickname.
        nick: String,
        /// The user's new real name.
        realname: String,
    },
    /// A user or channel mode changed.
    ModeChanged {
        /// The mode target.
        target: String,
        /// The user or server that changed the modes.
        by: String,
        /// Parsed mode operations in wire order.
        changes: Vec<OwnedModeChange>,
    },
    /// A channel topic changed live.
    TopicChanged {
        /// The affected channel.
        channel: String,
        /// The new topic, or `None` when it was cleared.
        topic: Option<String>,
        /// The user or server that changed the topic.
        by: String,
    },
    /// Server input exceeded a configured state-retention bound.
    StateLimitReached {
        /// The state collection that rejected another entry.
        limit: StateLimit,
    },
    /// A monitored nickname came online or changed its userhost.
    MonitorOnline {
        /// The monitored nickname.
        nick: String,
        /// The user's current username, when supplied by the server.
        user: Option<String>,
        /// The user's current hostname, when supplied by the server.
        host: Option<String>,
    },
    /// A monitored nickname went offline.
    MonitorOffline {
        /// The monitored nickname.
        nick: String,
    },
    /// The server rejected nicknames because its monitor list was full.
    MonitorListFull {
        /// The server's advertised monitor-list limit.
        limit: usize,
        /// Nicknames the server did not add.
        nicks: Vec<String>,
    },
    /// The server does not advertise MONITOR support.
    MonitorUnsupported,
    /// The connection ended.
    Disconnected {
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
        /// Whether the client will try to connect again.
        will_retry: bool,
    },
}

// This transient mapping keeps forwarding allocation-free for every event kind.
#[allow(clippy::large_enum_variant)]
pub(crate) enum CoreEvent {
    Forward(Event),
    Message(papo_core::ReceivedMessage),
    History {
        target: String,
        messages: Vec<papo_core::HistoryMessage>,
        end: bool,
    },
    Close {
        kind: papo_core::CloseKind,
        reason: String,
    },
    Unknown,
}

// Keeping the exhaustive forwarding table together makes omissions visible when core grows.
#[allow(clippy::too_many_lines)]
pub(crate) fn from_core(event: papo_core::Event) -> CoreEvent {
    CoreEvent::Forward(match event {
        papo_core::Event::Registered(welcome) => Event::Registered(welcome),
        papo_core::Event::Message(message) => {
            if message.as_ref().command_is("001") {
                return CoreEvent::Unknown;
            }
            return CoreEvent::Message(message);
        }
        papo_core::Event::MultilineMessage(message) => Event::Raw(message.message),
        papo_core::Event::TagMessage(message) => {
            let tag = TagMsg {
                source: message.source,
                target: message.target,
                tags: message.tags,
            };
            if tag.reaction().is_some() {
                Event::Reaction(tag)
            } else if tag.typing().is_some() {
                Event::Typing(tag)
            } else {
                Event::TagMsg(tag)
            }
        }
        papo_core::Event::Redacted(redacted) => Event::Redacted(redacted),
        papo_core::Event::Response(response) => Event::Response(response),
        papo_core::Event::RequestUntracked { id } => Event::RequestUntracked { id },
        papo_core::Event::RequestTimedOut { id } => Event::RequestTimedOut { id },
        papo_core::Event::RequestAborted { id } => Event::RequestAborted { id },
        papo_core::Event::StandardReply(reply) => Event::StandardReply(reply),
        papo_core::Event::AccountRegistered { account, logged_in } => {
            Event::AccountRegistered { account, logged_in }
        }
        papo_core::Event::VerificationRequired { account, message } => {
            Event::VerificationRequired { account, message }
        }
        papo_core::Event::PersistenceStatus { setting, effective } => {
            Event::PersistenceStatus { setting, effective }
        }
        papo_core::Event::MetadataChanged {
            target,
            key,
            value,
            visibility,
        } => Event::MetadataChanged {
            target,
            key,
            value,
            visibility,
        },
        papo_core::Event::MetadataSyncLater {
            target,
            retry_after,
        } => Event::MetadataSyncLater {
            target,
            retry_after,
        },
        papo_core::Event::History {
            target,
            messages,
            end,
        } => {
            return CoreEvent::History {
                target,
                messages,
                end,
            };
        }
        papo_core::Event::HistoryTargets(targets) => Event::HistoryTargets(targets),
        papo_core::Event::ReadMarker(marker) => Event::ReadMarker(marker),
        papo_core::Event::Netsplit { servers, quits } => Event::Netsplit { servers, quits },
        papo_core::Event::Netjoin { servers, joins } => Event::Netjoin { servers, joins },
        papo_core::Event::BatchAbandoned {
            reference,
            batch_type,
            reason,
        } => Event::BatchAbandoned {
            reference,
            batch_type,
            reason,
        },
        papo_core::Event::Malformed { line } => Event::Malformed { line },
        papo_core::Event::Close { kind, reason } => return CoreEvent::Close { kind, reason },
        papo_core::Event::AuthenticationFailed { reason } => Event::AuthenticationFailed { reason },
        papo_core::Event::CapsChanged { added, removed } => Event::CapsChanged { added, removed },
        papo_core::Event::StsUpgrade { port } => Event::StsUpgrade { port },
        papo_core::Event::StsPolicy(action) => Event::StsPolicy(action),
        papo_core::Event::Joined { channel, nick } => Event::Joined { channel, nick },
        papo_core::Event::ChannelSynced { channel } => Event::ChannelSynced { channel },
        papo_core::Event::Parted {
            channel,
            nick,
            reason,
        } => Event::Parted {
            channel,
            nick,
            reason,
        },
        papo_core::Event::Kicked {
            channel,
            nick,
            by,
            reason,
        } => Event::Kicked {
            channel,
            nick,
            by,
            reason,
        },
        papo_core::Event::Quit {
            nick,
            reason,
            channels,
        } => Event::Quit {
            nick,
            reason,
            channels,
        },
        papo_core::Event::NickChanged { old, new } => Event::NickChanged { old, new },
        papo_core::Event::ChannelRenamed { old, new, reason } => {
            Event::ChannelRenamed { old, new, reason }
        }
        papo_core::Event::AccountChanged { nick, account } => {
            Event::AccountChanged { nick, account }
        }
        papo_core::Event::AwayChanged { nick, message } => Event::AwayChanged { nick, message },
        papo_core::Event::HostChanged { nick, user, host } => {
            Event::HostChanged { nick, user, host }
        }
        papo_core::Event::RealnameChanged { nick, realname } => {
            Event::RealnameChanged { nick, realname }
        }
        papo_core::Event::ModeChanged {
            target,
            by,
            changes,
        } => Event::ModeChanged {
            target,
            by,
            changes,
        },
        papo_core::Event::TopicChanged { channel, topic, by } => {
            Event::TopicChanged { channel, topic, by }
        }
        papo_core::Event::StateLimitReached { limit } => Event::StateLimitReached { limit },
        papo_core::Event::MonitorOnline { nick, user, host } => {
            Event::MonitorOnline { nick, user, host }
        }
        papo_core::Event::MonitorOffline { nick } => Event::MonitorOffline { nick },
        papo_core::Event::MonitorListFull { limit, nicks } => {
            Event::MonitorListFull { limit, nicks }
        }
        papo_core::Event::MonitorUnsupported => Event::MonitorUnsupported,
        _ => return CoreEvent::Unknown,
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant, UNIX_EPOCH};

    use super::{CoreEvent, Event, MessageEvent, MessageKind, Origin, TagMsg, from_core};
    use papo_core::{
        Account, BatchAbandonReason, Capabilities, Config, OwnedModeChange, Session, StandardReply,
        StateLimit,
    };
    use papo_proto::{Isupport, Message, MessageRef, ModeKind, Source, StandardReplyKind, Typing};

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
        assert_eq!(
            chat("PRIVMSG", "\x01VERSION\x01").kind(),
            MessageKind::Ctcp("VERSION")
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
                nick: "emily",
                user: Some("~emily"),
                host: Some("localhost")
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
        assert_eq!(tagged.msgid(), Some("RfcBTzyxY120004"));
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
            matches!(forwarded(session.poll_event().unwrap()), Event::Response(response) if response.id == id)
        );
    }

    #[test]
    fn maps_multiline_as_a_normal_message() {
        let multiline = papo_core::MultilineMessage {
            message: Message::privmsg("#papo", "first\nsecond"),
        };
        assert!(matches!(
            forwarded(papo_core::Event::MultilineMessage(multiline)),
            Event::Raw(message) if message.as_ref().param(1) == Some("first\nsecond")
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
        let Event::Reaction(tag) = forwarded(papo_core::Event::TagMessage(tag)) else {
            panic!("expected reaction")
        };
        assert_eq!(tag.source(), Some("alice!~a@cloak"));
        assert_eq!(tag.target(), "#papo");
        assert_eq!(
            tag.tags(),
            [
                ("+typing".into(), Some("paused".into())),
                ("+draft/react".into(), Some("👍".into())),
                ("+reply".into(), Some("RKi9Cw".into())),
            ]
        );
        assert_eq!(tag.typing(), Some(Typing::Paused));
        assert!(
            tag.reaction()
                .is_some_and(|reaction| reaction.reply_to == "RKi9Cw")
        );
    }

    #[test]
    fn distinguishes_every_typing_tag_value() {
        let typing = |value: &str| {
            TagMsg {
                source: None,
                target: "#papo".into(),
                tags: vec![("+typing".into(), Some(value.into()))],
            }
            .typing()
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
            matches!(forwarded(papo_core::Event::TagMessage(tag)), Event::Typing(tag) if tag.typing() == Some(Typing::Active))
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
            Event::Redacted(mapped) if mapped == redacted
        ));
    }

    #[test]
    fn maps_request_untracked() {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        let id = session
            .request(now, Message::new("MODE", ["#papo"]))
            .unwrap();
        assert!(
            matches!(forwarded(session.poll_event().unwrap()), Event::RequestUntracked { id: mapped } if mapped == id)
        );
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
            matches!(forwarded(session.poll_event().unwrap()), Event::RequestTimedOut { id: mapped } if mapped == id)
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
            matches!(forwarded(session.poll_event().unwrap()), Event::RequestAborted { id: mapped } if mapped == id)
        );
    }

    #[test]
    fn maps_standard_reply() {
        let reply = StandardReply {
            kind: StandardReplyKind::Warn,
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
        assert_eq!(mapped, reply);
    }

    #[test]
    fn maps_account_registration_results() {
        assert!(matches!(
            forwarded(papo_core::Event::AccountRegistered {
                account: "alice".into(),
                logged_in: true,
            }),
            Event::AccountRegistered { account, logged_in: true } if account == "alice"
        ));
        assert!(matches!(
            forwarded(papo_core::Event::VerificationRequired {
                account: "alice".into(),
                message: "Verification code sent".into(),
            }),
            Event::VerificationRequired { account, message }
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
            Event::ChannelRenamed { old, new, reason }
                if old == "#rust" && new == "#rust-lang" && reason == "Project renamed"
        ));
    }

    #[test]
    fn maps_persistence_status() {
        assert!(matches!(
            forwarded(papo_core::Event::PersistenceStatus {
                setting: papo_core::PersistenceSetting::Default,
                effective: true,
            }),
            Event::PersistenceStatus {
                setting: papo_core::PersistenceSetting::Default,
                effective: true,
            }
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
            Event::MetadataChanged { target, key, value: Some(value), visibility }
                if target == "#papo" && key == "pronouns"
                    && value == "they/them" && visibility == "members"
        ));
        assert!(matches!(
            forwarded(papo_core::Event::MetadataSyncLater {
                target: "alice".into(),
                retry_after: Some(Duration::from_secs(90)),
            }),
            Event::MetadataSyncLater { target, retry_after: Some(delay) }
                if target == "alice" && delay == Duration::from_secs(90)
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
            Event::Joined { channel, nick } if channel == "#papo" && nick == "emily"
        ));
    }

    #[test]
    fn maps_channel_synced() {
        assert!(matches!(
            forwarded(papo_core::Event::ChannelSynced {
                channel: "#papo".into()
            }),
            Event::ChannelSynced { channel } if channel == "#papo"
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
            Event::Parted { channel, nick, reason }
                if channel == "#papo" && nick == "emily" && reason.as_deref() == Some("Leaving for lunch")
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
            Event::Kicked { channel, nick, by, reason }
                if channel == "#papo" && nick == "emily" && by == "alice"
                    && reason.as_deref() == Some("Flooding")
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
            Event::Quit { nick, reason, channels }
                if nick == "emily" && reason.as_deref() == Some("Client exited")
                    && channels == ["#papo", "#rust"]
        ));
    }

    #[test]
    fn maps_nick_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::NickChanged {
                old: "emily".into(),
                new: "em".into(),
            }),
            Event::NickChanged { old, new } if old == "emily" && new == "em"
        ));
    }

    #[test]
    fn maps_account_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::AccountChanged {
                nick: "emily".into(),
                account: Account::LoggedIn("emily".into()),
            }),
            Event::AccountChanged { nick, account }
                if nick == "emily" && account == Account::LoggedIn("emily".into())
        ));
    }

    #[test]
    fn maps_away_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::AwayChanged {
                nick: "emily".into(),
                message: Some("Gone to lunch".into()),
            }),
            Event::AwayChanged { nick, message }
                if nick == "emily" && message.as_deref() == Some("Gone to lunch")
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
            Event::HostChanged { nick, user, host }
                if nick == "emily" && user == "~em" && host == "users/emily"
        ));
    }

    #[test]
    fn maps_realname_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::RealnameChanged {
                nick: "emily".into(),
                realname: "Emily Smith".into(),
            }),
            Event::RealnameChanged { nick, realname }
                if nick == "emily" && realname == "Emily Smith"
        ));
    }

    #[test]
    fn maps_mode_changed() {
        let changes = vec![OwnedModeChange {
            set: true,
            mode: 'o',
            arg: Some("emily".into()),
            kind: ModeKind::Prefix(0),
        }];
        assert!(matches!(
            forwarded(papo_core::Event::ModeChanged {
                target: "#papo".into(),
                by: "alice".into(),
                changes: changes.clone(),
            }),
            Event::ModeChanged { target, by, changes: forwarded_changes }
                if target == "#papo" && by == "alice" && forwarded_changes == changes
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
            Event::TopicChanged { channel, topic, by }
                if channel == "#papo"
                    && topic.as_deref() == Some("Rust 1.90 is out")
                    && by == "alice"
        ));
    }

    #[test]
    fn maps_state_limit_reached() {
        assert!(matches!(
            forwarded(papo_core::Event::StateLimitReached {
                limit: StateLimit::Members
            }),
            Event::StateLimitReached {
                limit: StateLimit::Members
            }
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
            Event::MonitorOnline { nick, user, host }
                if nick == "alice" && user.as_deref() == Some("~alice") && host.as_deref() == Some("cloak.example")
        ));
        assert!(matches!(
            forwarded(papo_core::Event::MonitorOnline {
                nick: "bob".into(),
                user: None,
                host: None,
            }),
            Event::MonitorOnline { nick, user: None, host: None } if nick == "bob"
        ));
    }

    #[test]
    fn maps_monitor_offline() {
        assert!(matches!(
            forwarded(papo_core::Event::MonitorOffline {
                nick: "alice".into(),
            }),
            Event::MonitorOffline { nick } if nick == "alice"
        ));
    }

    #[test]
    fn maps_monitor_list_full() {
        assert!(matches!(
            forwarded(papo_core::Event::MonitorListFull {
                limit: 100,
                nicks: vec!["alice".into(), "bob".into()],
            }),
            Event::MonitorListFull { limit, nicks }
                if limit == 100 && nicks == ["alice", "bob"]
        ));
    }

    #[test]
    fn maps_monitor_unsupported() {
        assert!(matches!(
            forwarded(papo_core::Event::MonitorUnsupported),
            Event::MonitorUnsupported
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
            messages: vec![papo_core::HistoryMessage {
                message: message.clone(),
                context: false,
            }],
            end: true,
        })
        else {
            panic!("history was not classified")
        };
        assert_eq!(target, "#papo");
        assert_eq!(messages[0].message, message);
        assert!(!messages[0].context);
        assert!(end);
    }

    #[test]
    fn maps_history_targets() {
        let targets = vec![("#papo".into(), UNIX_EPOCH + Duration::from_secs(42))];
        assert!(matches!(
            forwarded(papo_core::Event::HistoryTargets(targets.clone())),
            Event::HistoryTargets(mapped) if mapped == targets
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
            Event::ReadMarker(mapped) if mapped == marker
        ));
    }

    #[test]
    fn maps_netsplit() {
        let quit = Message::new("QUIT", ["irc.example irc.remote"]);
        let Event::Netsplit { servers, quits } = forwarded(papo_core::Event::Netsplit {
            servers: ["irc.example".into(), "irc.remote".into()],
            quits: vec![quit.clone()],
        }) else {
            panic!("netsplit was not forwarded")
        };
        assert_eq!(servers, ["irc.example", "irc.remote"]);
        assert_eq!(quits, vec![quit]);
    }

    #[test]
    fn maps_netjoin() {
        let join = Message::join("#papo").with_source("alice!~a@remote.example");
        let Event::Netjoin { servers, joins } = forwarded(papo_core::Event::Netjoin {
            servers: ["irc.example".into(), "irc.remote".into()],
            joins: vec![join.clone()],
        }) else {
            panic!("netjoin was not forwarded")
        };
        assert_eq!(servers, ["irc.example", "irc.remote"]);
        assert_eq!(joins, vec![join]);
    }

    #[test]
    fn maps_batch_abandoned() {
        let Event::BatchAbandoned {
            reference,
            batch_type,
            reason,
        } = forwarded(papo_core::Event::BatchAbandoned {
            reference: "history42".into(),
            batch_type: "chathistory".into(),
            reason: BatchAbandonReason::Timeout,
        })
        else {
            panic!("abandonment was not forwarded")
        };
        assert_eq!(reference, "history42");
        assert_eq!(batch_type, "chathistory");
        assert_eq!(reason, BatchAbandonReason::Timeout);
    }
}
