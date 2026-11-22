use std::{
    future::IntoFuture,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use papo_core::{Response, ResponseMatcher, TextKind};
use papo_proto::{Ctcp, Line, Typing};
use tokio::sync::{mpsc, oneshot};

use crate::{
    Error, Feature, InvalidInput, Unsupported, UnsupportedReason,
    driver::{Command, CoreRequest},
    event_impl,
    state::State,
    target::sealed::SendName,
};

static NEXT_PING: AtomicU64 = AtomicU64::new(0);

/// A cheap, cloneable handle to a running IRC client.
///
/// Typed requests use server labels when available and otherwise match protocol
/// reply boundaries. Concurrent identical unlabelled requests resolve in FIFO order.
///
/// ```no_run
/// # async fn send() -> Result<(), papo::Error> {
/// let (client, _) = papo::Client::builder("ircs://irc.example.net", "papo-client")
///     .connect()
///     .await?;
/// client.privmsg("#papo", "hello from the docs").await?;
/// client.quit().reason("example complete").await?;
/// # Ok(())
/// # }
/// ```
///
/// Dropping one clone has no connection effect. The connection stays alive while any `Client` or
/// its [`crate::Events`] stream exists. Dropping the final handle asks the driver to send `QUIT`,
/// but `Drop` cannot await delivery or the server's acknowledgement. Call [`Client::quit`] for an
/// orderly application shutdown.
#[derive(Clone)]
pub struct Client {
    pub(crate) commands: mpsc::Sender<Command>,
    pub(crate) state: Arc<RwLock<Arc<State>>>,
    pub(crate) origin: crate::Origin,
    pub(crate) network_builder: Option<Arc<RwLock<crate::builder::NetworkBuilderSettings>>>,
}

/// What papo sent for a chat message, line by line, and what the server confirmed of it.
///
/// Applications that log every chat message must log their own sends from this value. The
/// correlated server copy is consumed and is not also emitted as a [`crate::Message`] event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SendOutcome {
    conversation: crate::Conversation,
    audience: Option<crate::Prefix>,
    pub(crate) receipt: crate::message_ref::Receipt,
    /// Each logical line papo sent, in send order, with the server's confirmation of it.
    ///
    /// Long text is split into several lines, and segments in one `draft/multiline` batch form
    /// one logical line. Each line is confirmed by its own echo, so when only some echoes carry
    /// a `msgid`, the others are [`crate::Confirmation::Unconfirmed`].
    pub lines: Vec<SentLine>,
    /// Compatibility fallbacks applied while sending.
    pub fallbacks: Vec<Fallback>,
}

/// One logical line of a [`SendOutcome`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SentLine {
    /// The exact text papo sent, with a batch's line breaks restored as `\n`.
    pub text: String,
    /// Whether the server confirmed this line, with the identity it assigned when it did.
    pub confirmation: crate::Confirmation,
}

impl SentLine {
    pub(crate) fn unconfirmed(texts: Vec<String>, reason: crate::UnconfirmedReason) -> Vec<Self> {
        texts
            .into_iter()
            .map(|text| Self {
                text,
                confirmation: crate::Confirmation::unconfirmed(reason),
            })
            .collect()
    }
}

/// The result of adding or removing a reaction with [`Client::react`] or [`Client::unreact`].
///
/// The labelled echo of the reaction is consumed here and is not also emitted as an event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ReactOutcome {
    /// IRC conversation to which the reaction was sent.
    ///
    /// See [`crate::Conversation`] for status-prefixed line guidance.
    pub conversation: crate::Conversation,
    /// The lowest-ranked `STATUSMSG` prefix on the wire address.
    ///
    /// See [`crate::Conversation`] for status-prefixed line guidance.
    pub status: Option<crate::Prefix>,
    /// Whether the server confirmed the reaction, with the identity it assigned when it did.
    pub confirmation: crate::Confirmation,
}

/// The result of redacting a message with [`Client::redact`].
///
/// The server's echo of each redaction is also emitted as [`crate::Event::Redacted`], before the
/// request resolves.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RedactOutcome {
    /// IRC conversation in which the message was redacted.
    ///
    /// See [`crate::Conversation`] for status-prefixed line guidance.
    pub conversation: crate::Conversation,
    /// The lowest-ranked `STATUSMSG` prefix on the wire address.
    ///
    /// See [`crate::Conversation`] for status-prefixed line guidance.
    pub status: Option<crate::Prefix>,
    /// Each line papo redacted, in send order, with the server's confirmation of it.
    ///
    /// A message sent as several lines has one entry per line.
    pub lines: Vec<RedactedLine>,
}

/// One redacted line of a [`RedactOutcome`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RedactedLine {
    /// The server message identifier of the line that was redacted.
    pub redacts: crate::MessageId,
    /// Whether the server confirmed the redaction, with the identity it gave the `REDACT` echo.
    pub confirmation: crate::Confirmation,
}

/// A compatibility fallback applied to an outgoing chat message.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Fallback {
    /// The reply tag was unavailable or denied by the server.
    ReplyTagDropped,
    /// The channel-context tag was unavailable or denied by the server.
    ContextTagDropped,
    /// The text was divided into multiple IRC message lines.
    SplitIntoLines,
}

/// Builds a chat message with optional `IRCv3` client tags.
///
/// ```compile_fail
/// # #![deny(unused_must_use)]
/// # fn send(client: &papo::Client) {
/// client.privmsg("#papo", "hello from the docs");
/// # }
/// ```
///
/// Threading under a message goes through [`Client::reply`], which answers that line's own
/// audience. A bare message identifier cannot thread a send, because nothing would tie it to the
/// audience that saw the message:
///
/// ```compile_fail,E0624
/// # async fn send(client: &papo::Client, id: papo::MessageId) -> Result<(), papo::Error> {
/// client.privmsg("#ops", "fixed").reply_to(id).await?;
/// # Ok(()) }
/// ```
#[must_use = "requests do nothing unless awaited"]
pub struct SendMessage {
    client: Client,
    kind: TextKind,
    target: crate::target::sealed::Parts<'static>,
    text: String,
    reply_to: Option<crate::MessageId>,
    channel_context: Option<crate::target::sealed::Parts<'static>>,
    tags: Vec<(String, String)>,
    action: bool,
    invalid_input: Option<InvalidInput>,
}

struct ResolvedSend {
    text: String,
    guard: Option<papo_core::TargetGuard>,
    person: Option<papo_core::Proven>,
}

impl ResolvedSend {
    fn conversation(
        &self,
        client: &Client,
        state: &State,
    ) -> Result<(Option<crate::Prefix>, crate::Conversation), Error> {
        let (status, core_conversation) = state
            .parse_target_inner(&self.text)
            .map_err(|reason| InvalidInput::invalid_name(&self.text, reason))?;
        let channel_guard = match (&core_conversation, self.guard.as_ref()) {
            (_, Some(papo_core::TargetGuard::Channel(channel))) => Some(channel.clone()),
            (papo_core::Conversation::Channel(channel), _)
                if state.channel_local(channel).is_some() =>
            {
                Some(state.core.__channel_guard(channel))
            }
            _ => None,
        };
        let conversation = crate::Conversation::from_core(
            core_conversation,
            client.origin,
            Some(&client.state),
            channel_guard,
        );
        Ok((status, conversation))
    }

    fn into_chat_guard(self) -> crate::driver::ChatGuard {
        self.person.map_or_else(
            || crate::driver::ChatGuard::Target(self.guard),
            crate::driver::ChatGuard::Person,
        )
    }
}

impl std::fmt::Debug for SendMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("SendMessage", formatter)
    }
}

macro_rules! reason_step {
    ($request:ty, $action:literal) => {
        impl $request {
            #[doc = concat!("Sets the human-readable ", $action, " reason; an empty reason sends none.")]
            pub fn reason(mut self, reason: impl Into<String>) -> Self {
                self.reason = reason.into();
                self
            }
        }
    };
}
pub(crate) use reason_step;

/// Builds a channel join request.
#[must_use = "requests do nothing unless awaited"]
pub struct JoinChannel {
    client: Client,
    channel: crate::target::sealed::Parts<'static>,
    key: Option<crate::Secret>,
}

/// Starts a kickban request; name the nickname with [`Kickban::nick`].
///
/// A kickban without its nickname and ban cannot be awaited:
///
/// ```compile_fail
/// # async fn request(client: &papo::Client) -> Result<(), papo::Error> {
/// client.kickban("#papo").await?;
/// # Ok(())
/// # }
/// ```
#[must_use = "requests do nothing unless awaited"]
pub struct Kickban {
    client: Client,
    channel: crate::target::sealed::Parts<'static>,
}

/// A kickban request with its nickname; name the ban with [`KickbanNick::ban`].
///
/// A kickban without its ban cannot be awaited:
///
/// ```compile_fail
/// # async fn request(client: &papo::Client) -> Result<(), papo::Error> {
/// client.kickban("#papo").nick("mallory").await?;
/// # Ok(())
/// # }
/// ```
#[must_use = "requests do nothing unless awaited"]
pub struct KickbanNick {
    client: Client,
    channel: crate::target::sealed::Parts<'static>,
    nick: crate::target::sealed::Parts<'static>,
}

/// A complete kickban request; awaiting it yields a [`crate::KickbanOutcome`].
///
/// Naming each role keeps a nickname and a ban mask, both often plain text, from being swapped.
#[must_use = "requests do nothing unless awaited"]
pub struct KickbanRequest {
    client: Client,
    channel: crate::target::sealed::Parts<'static>,
    nick: crate::target::sealed::Parts<'static>,
    ban: crate::BanTarget,
    reason: String,
}

/// A channel departure request; add an optional reason with [`PartRequest::reason`].
#[must_use = "requests do nothing unless awaited"]
pub struct PartRequest {
    client: Client,
    channel: crate::target::sealed::Parts<'static>,
    reason: String,
}

impl std::fmt::Debug for PartRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("PartRequest", formatter)
    }
}

reason_step!(PartRequest, "part");

impl IntoFuture for PartRequest {
    type Output = Result<crate::PartOutcome, Error>;
    type IntoFuture = crate::RequestFuture<crate::PartOutcome>;

    fn into_future(self) -> Self::IntoFuture {
        self.client.part_request(self.channel, self.reason)
    }
}

/// A channel removal request; add an optional reason with [`KickRequest::reason`].
#[must_use = "requests do nothing unless awaited"]
pub struct KickRequest {
    client: Client,
    channel: crate::target::sealed::Parts<'static>,
    nick: crate::target::sealed::Parts<'static>,
    reason: String,
}

impl std::fmt::Debug for KickRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("KickRequest", formatter)
    }
}

reason_step!(KickRequest, "kick");

impl IntoFuture for KickRequest {
    type Output = Result<crate::KickOutcome, Error>;
    type IntoFuture = crate::RequestFuture<crate::KickOutcome>;

    fn into_future(self) -> Self::IntoFuture {
        self.client
            .kick_request(self.channel, self.nick, self.reason)
    }
}

/// A clean disconnect request; add an optional reason with [`QuitRequest::reason`].
#[must_use = "requests do nothing unless awaited"]
pub struct QuitRequest {
    client: Client,
    reason: String,
}

impl std::fmt::Debug for QuitRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("QuitRequest", formatter)
    }
}

reason_step!(QuitRequest, "quit");

impl IntoFuture for QuitRequest {
    type Output = Result<(), Error>;
    type IntoFuture = crate::RequestFuture<()>;

    fn into_future(self) -> Self::IntoFuture {
        self.client.quit_request(self.reason)
    }
}

impl std::fmt::Debug for JoinChannel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("JoinChannel", formatter)
    }
}

impl std::fmt::Debug for Kickban {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("Kickban", formatter)
    }
}

impl std::fmt::Debug for KickbanNick {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("KickbanNick", formatter)
    }
}

impl std::fmt::Debug for KickbanRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("KickbanRequest", formatter)
    }
}

impl JoinChannel {
    /// Supplies the channel key without exposing it through `Debug` output.
    pub fn key(mut self, key: crate::Secret) -> Self {
        self.key = Some(key);
        self
    }
}

impl Kickban {
    /// Names the nickname to remove after the ban is confirmed.
    pub fn nick(self, nick: impl crate::AsNick) -> KickbanNick {
        KickbanNick {
            client: self.client,
            channel: self.channel,
            nick: crate::AsNick::__parts(&nick).into_owned(),
        }
    }
}

impl KickbanNick {
    /// Names the ban target to confirm before removing the nickname.
    pub fn ban(self, target: impl Into<crate::BanTarget>) -> KickbanRequest {
        KickbanRequest {
            client: self.client,
            channel: self.channel,
            nick: self.nick,
            ban: target.into(),
            reason: String::new(),
        }
    }
}

reason_step!(KickbanRequest, "kick");

impl IntoFuture for JoinChannel {
    type Output = Result<crate::ChannelRef, Error>;
    type IntoFuture = crate::RequestFuture<crate::ChannelRef>;

    fn into_future(self) -> Self::IntoFuture {
        crate::RequestFuture::new(async move {
            let channel = self
                .client
                .resolve(crate::target::Role::Channel("join: channel"), self.channel)?;
            let channel = self
                .client
                .state()
                .known_name_rules()
                .ok_or(InvalidInput::NameRulesUnknown)?
                .channel(channel);
            self.client.join_channel(channel, self.key).await
        })
    }
}

impl IntoFuture for KickbanRequest {
    type Output = Result<crate::KickbanOutcome, Error>;
    type IntoFuture = crate::RequestFuture<crate::KickbanOutcome>;

    fn into_future(self) -> Self::IntoFuture {
        self.client
            .kickban_request(self.channel, self.nick, self.ban, self.reason)
    }
}

impl SendMessage {
    pub(crate) fn reply_to(mut self, message: impl Into<crate::MessageId>) -> Self {
        self.reply_to = Some(message.into());
        self
    }

    /// Supplies channel context for a private message.
    pub fn channel_context(mut self, channel: impl crate::AsChannel) -> Self {
        self.channel_context = Some(crate::AsChannel::__parts(&channel).into_owned());
        self
    }

    /// Adds a client-only message tag.
    ///
    /// The reply, reaction, and channel-context keys in [`crate::tags::keys`] are managed by
    /// typed methods; setting them here can conflict with those methods.
    ///
    /// # Errors
    ///
    /// A key without the client-only `+` prefix returns [`Error::InvalidInput`]. If
    /// `message-tags` is unavailable, this returns [`Error::Unsupported`] for
    /// [`Feature::MessageTags`] with [`UnsupportedReason::NotOffered`] or
    /// [`UnsupportedReason::DisabledByClient`]. A key rejected by `CLIENTTAGDENY` returns the
    /// same feature with [`UnsupportedReason::TagDenied`]. Other key grammar and encoded-size
    /// failures return [`Error::InvalidInput`].
    /// A send that widens a remembered status-prefixed line's audience returns
    /// [`InvalidInput::WiderThanReferenced`].
    pub fn tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.push((key.into(), value.into()));
        self
    }
}

impl IntoFuture for SendMessage {
    type Output = Result<SendOutcome, Error>;
    type IntoFuture = crate::RequestFuture<SendOutcome>;

    fn into_future(self) -> Self::IntoFuture {
        crate::RequestFuture::new(async move {
            if let Some(error) = self.invalid_input {
                return Err(error.into());
            }
            let target = self.client.resolve_send("target", self.target)?;
            let channel_context = self
                .channel_context
                .map(|channel| {
                    self.client.resolve(
                        crate::target::Role::Channel("channel_context: channel"),
                        channel,
                    )
                })
                .transpose()?;
            let state = self.client.state();
            for (key, _) in &self.tags {
                if !key.starts_with('+') {
                    return Err(InvalidInput::InvalidClientTagKey(key.clone()).into());
                }
            }
            if !self.tags.is_empty() {
                Client::require_feature(&state, Feature::MessageTags)?;
                let denied = state.isupport().clienttagdeny();
                if self.tags.iter().any(|(key, _)| denied.denies(key)) {
                    return Err(Error::Unsupported(Unsupported::new(
                        Feature::MessageTags,
                        UnsupportedReason::TagDenied,
                    )));
                }
            }
            if let Some(message) = &self.reply_to {
                message
                    .check()
                    .map_err(|reason| InvalidInput::invalid_name(message.to_string(), reason))?;
            }
            let mut tags = self.tags;
            let mut fallbacks = Vec::new();
            if state.caps().contains_key(&crate::cap::MESSAGE_TAGS) {
                let denied = state.isupport().clienttagdeny();
                if let Some(message) = self.reply_to.as_ref()
                    && let Some(tag) = event_impl::allowed_alias(&denied, "+reply", "+draft/reply")
                {
                    tags.push((tag.to_owned(), message.to_string()));
                } else if self.reply_to.is_some() {
                    fallbacks.push(Fallback::ReplyTagDropped);
                }
                if let Some(channel) = channel_context.as_ref()
                    && let Some(tag) = event_impl::allowed_alias(
                        &denied,
                        "+channel-context",
                        "+draft/channel-context",
                    )
                {
                    tags.push((tag.to_owned(), channel.clone()));
                } else if channel_context.is_some() {
                    fallbacks.push(Fallback::ContextTagDropped);
                }
            } else {
                fallbacks.extend(self.reply_to.map(|_| Fallback::ReplyTagDropped));
                fallbacks.extend(channel_context.map(|_| Fallback::ContextTagDropped));
            }
            self.client
                .send_text(self.kind, target, &self.text, tags, fallbacks, self.action)
                .await
        })
    }
}

impl SendOutcome {
    /// Returns the sent message's conversation, kept read-only to match its return address.
    ///
    /// See [`crate::Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub const fn conversation(&self) -> &crate::Conversation {
        &self.conversation
    }

    /// Returns the lowest-ranked `STATUSMSG` prefix, kept read-only to preserve the sent audience.
    ///
    /// See [`crate::Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub const fn audience(&self) -> Option<crate::Prefix> {
        self.audience
    }

    /// Which `Client` produced this value (not the sender or the server).
    #[must_use]
    pub fn origin(&self) -> crate::Origin {
        crate::message_ref::conversation_origin(&self.conversation)
    }

    /// Returns the address that reaches the recipients of the sent message.
    ///
    /// See [`crate::Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn return_address(&self) -> crate::MessageRef {
        crate::MessageRef::from(self)
    }

    pub(crate) fn new(
        conversation: crate::Conversation,
        audience: Option<crate::Prefix>,
        receipt: crate::message_ref::Receipt,
        fallbacks: Vec<Fallback>,
        lines: Vec<SentLine>,
    ) -> Self {
        Self {
            conversation,
            audience,
            receipt,
            lines,
            fallbacks,
        }
    }

    /// `echoes` holds one entry per logical line: the labelled echo, if its response was one.
    pub(crate) fn from_echoes(
        echoes: &[Option<papo_core::SentInfo>],
        texts: Vec<String>,
        conversation: crate::Conversation,
        audience: Option<crate::Prefix>,
        mut receipt: crate::message_ref::Receipt,
        fallbacks: Vec<Fallback>,
    ) -> Self {
        let msgids = echoes
            .iter()
            .flatten()
            .map(|sent| crate::MessageId::from(sent.msgid.clone()))
            .collect::<Vec<_>>();
        if msgids.len() == texts.len() {
            receipt.set_reference(msgids, None);
        }
        debug_assert_eq!(echoes.len(), texts.len(), "one response per logical line");
        // papo labels a send only when echo-message is on as well.
        let lines = texts
            .into_iter()
            .zip(echoes)
            .map(|(text, echo)| SentLine {
                text,
                confirmation: crate::Confirmation::from_labelled_echo(true, echo.clone()),
            })
            .collect();
        Self {
            conversation,
            audience,
            receipt,
            lines,
            fallbacks,
        }
    }
}

/// Servers disagree on stacked signs: Ergo answers `+@#ops` with 401 and Solanum widens it to
/// voiced members, so only one sign can name an audience reliably.
fn refuse_stacked_status(state: &State, text: &str) -> Result<(), crate::InvalidIdentifier> {
    let mut rest = text.chars();
    rest.next();
    match state.parse_target_inner(rest.as_str()) {
        Ok((Some(_), _)) => Err(crate::InvalidIdentifier::InvalidPrefix(
            rest.next().unwrap_or_default(),
        )),
        _ => Ok(()),
    }
}

fn current_channel_text(
    state: &State,
    text: String,
    channel: Option<&papo_core::ChannelGuard>,
) -> Result<String, Error> {
    if let Some(channel) = channel {
        let current = state.resolve_channel(channel).ok_or(Error::TargetGone)?;
        if current != *channel.__channel() {
            let prefix = text
                .strip_suffix(channel.__channel().as_str())
                .ok_or(Error::TargetGone)?;
            return Ok(format!("{prefix}{current}"));
        }
    }
    Ok(text)
}

impl Client {
    fn resolve_send(
        &self,
        argument: &'static str,
        parts: crate::target::sealed::Parts<'static>,
    ) -> Result<ResolvedSend, Error> {
        if let Some(handle) = parts.user.as_ref() {
            let proven = crate::identity::prove(self, handle)?;
            return Ok(ResolvedSend {
                text: proven.nick.to_string(),
                guard: None,
                person: Some(proven.core),
            });
        }
        crate::ForeignReference::require_local(parts.origin, self.origin)?;
        let state = self.state();
        let names = state
            .known_name_rules()
            .ok_or(InvalidInput::NameRulesUnknown)?;
        let text = current_channel_text(&state, parts.text.into_owned(), parts.channel.as_deref())?;
        let checked = || {
            if matches!(
                parts.send_name,
                Some(SendName::Nick | SendName::HistoryPeer)
            ) {
                let nick = names.nick(&text);
                nick.check(state.isupport().inner())?;
                return Ok(papo_core::Conversation::Private(nick));
            }
            let (status, target) = state.parse_target_inner(&text)?;
            if status.is_some() {
                refuse_stacked_status(&state, &text)?;
            }
            Ok(target)
        };
        let target =
            checked().map_err(|reason| InvalidInput::invalid_argument(argument, &text, reason))?;
        let person = match target {
            papo_core::Conversation::Private(nick)
                if parts.send_name == Some(SendName::HistoryPeer) =>
            {
                Some(
                    state
                        .core
                        .__prove_history_peer(nick)
                        .map_err(crate::identity::map_failure)?,
                )
            }
            papo_core::Conversation::Private(nick) if parts.send_name.is_some() => {
                Some(crate::identity::prove_current_name(&state, nick)?.core)
            }
            papo_core::Conversation::Private(_) => {
                let reason = crate::InvalidIdentifier::InvalidPrefix(
                    text.chars().next().unwrap_or_default(),
                );
                return Err(InvalidInput::invalid_argument(argument, text, reason).into());
            }
            _ => None,
        };
        Ok(ResolvedSend {
            text,
            guard: parts
                .channel
                .map(|channel| papo_core::TargetGuard::Channel(*channel)),
            person,
        })
    }

    pub(crate) fn resolve<'a>(
        &self,
        role: crate::target::Role,
        parts: impl Into<crate::target::sealed::Parts<'a>>,
    ) -> Result<String, Error> {
        let parts = parts.into();
        crate::ForeignReference::require_local(parts.origin, self.origin)?;
        let state = self.state();
        let name_rules = state
            .known_name_rules()
            .ok_or(InvalidInput::NameRulesUnknown)?;
        let mut text =
            current_channel_text(&state, parts.text.into_owned(), parts.channel.as_deref())?;
        if matches!(role, crate::target::Role::Nick(_))
            && let Some(proof) = parts.user.as_ref().and_then(crate::UserRef::proof)
        {
            text = state
                .core
                .__resolve_proven(proof)
                .map_err(crate::identity::map_failure)?
                .to_string();
        }
        let result = match role {
            crate::target::Role::Channel(_) => {
                name_rules.channel(&text).check(state.isupport().inner())
            }
            crate::target::Role::Nick(_) => name_rules.nick(&text).check(state.isupport().inner()),
            crate::target::Role::Account(_) => name_rules.account(&text).check(),
            crate::target::Role::Conversation(_) => {
                state.parse_target_inner(&text).and_then(|(status, _)| {
                    status.map(crate::Prefix::symbol).map_or(Ok(()), |prefix| {
                        Err(crate::InvalidIdentifier::InvalidPrefix(prefix))
                    })
                })
            }
            crate::target::Role::Mask(_) => {
                if let Some(prefix) = text.chars().next().filter(|prefix| {
                    state
                        .isupport()
                        .statusmsg()
                        .is_some_and(|statusmsg| statusmsg.contains(*prefix))
                }) {
                    Err(crate::InvalidIdentifier::InvalidPrefix(prefix))
                } else {
                    name_rules.mask(&text).check()
                }
            }
        };
        result.map_err(|reason| {
            InvalidInput::invalid_argument(role.argument(), text.clone(), reason)
        })?;
        Ok(text)
    }

    /// Which `Client` produced this value (not the sender or the server).
    #[must_use]
    pub const fn origin(&self) -> crate::Origin {
        self.origin
    }

    /// Starts configuring a client for `endpoint` with the primary `nick`.
    ///
    /// String endpoints must start with `irc://`, `ircs://`, `ws://`, or `wss://`.
    /// Use `ircs://` for IRC over TLS; bare hostnames are rejected.
    ///
    /// ```
    /// let builder = papo::Client::builder("ircs://irc.example.net", "papo-bot");
    /// # _ = builder;
    /// ```
    pub fn builder(endpoint: impl crate::IntoEndpoint, nick: impl crate::AsNick) -> crate::Builder {
        crate::Builder::from_endpoint(endpoint, nick)
    }

    /// Returns the latest immutable state snapshot.
    ///
    /// Use this for code outside an event handler. Inside an event handler, use the state returned
    /// by [`crate::Events::next`] so it cannot include later events.
    /// The returned snapshot can be held across `.await` points.
    #[must_use]
    pub fn snapshot(&self) -> Arc<State> {
        Arc::clone(
            &self
                .state
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    pub(crate) fn state(&self) -> Arc<State> {
        self.snapshot()
    }

    pub(crate) fn require_feature(state: &State, feature: Feature) -> Result<(), Error> {
        match state.unsupported_reason(feature) {
            Some(reason) => Err(Error::Unsupported(Unsupported::new(feature, reason))),
            None => Ok(()),
        }
    }
    /// Retries a disconnected connection immediately without waiting for its backoff.
    ///
    /// This never drops a live connection.
    ///
    /// ```no_run
    /// # async fn resume(client: &papo::Client) -> Result<(), papo::Error> {
    /// client.reconnect().await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::AlreadyConnected`] while connected, or [`Error::Disconnected`] after the
    /// driver stops.
    pub fn reconnect(&self) -> crate::RequestFuture<()> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let (result, receiver) = oneshot::channel();
            client
                .commands
                .send(Command::Reconnect { result })
                .await
                .map_err(|_| Error::connection_lost())?;
            receiver.await.map_err(|_| Error::connection_lost())?
        })
    }

    /// Pauses automatic reconnect attempts without dropping a live connection.
    ///
    /// If the connection drops while paused, no attempt starts until [`Self::resume_reconnect`].
    /// Calling this while already paused succeeds without changing anything.
    pub fn pause_reconnect(&self) -> crate::RequestFuture<()> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let (result, receiver) = oneshot::channel();
            client
                .commands
                .send(Command::PauseReconnect { result })
                .await
                .map_err(|_| Error::connection_lost())?;
            receiver.await.map_err(|_| Error::connection_lost())?
        })
    }

    /// Resumes automatic reconnect attempts and retries a disconnected connection immediately.
    ///
    /// Calling this while retries are already active succeeds without changing the current
    /// connection or backoff.
    pub fn resume_reconnect(&self) -> crate::RequestFuture<()> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let (result, receiver) = oneshot::channel();
            client
                .commands
                .send(Command::ResumeReconnect { result })
                .await
                .map_err(|_| Error::connection_lost())?;
            receiver.await.map_err(|_| Error::connection_lost())?
        })
    }

    /// Queues one IRC protocol line for transmission without typed target guards.
    ///
    /// The future resolves only after the entire line has been written.
    /// Sending is subject to the client's normal outbound rate limit.
    /// Construct the [`crate::Line`] with [`crate::Line::new`] and [`crate::Line::param`], or
    /// explicitly parse text for a `/quote`-style input. Parsed parameters are ordinary text: an application that puts a
    /// credential in a parsed raw line chooses readable diagnostics and no secret-buffer wiping.
    /// Prefer a typed client method or pass [`crate::Secret`] to [`crate::Line::param`] for credentials.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] when the line cannot be represented on
    /// the wire, or [`Error::Disconnected`] if the driver stops before writing it.
    /// A send that widens a remembered status-prefixed line's audience returns
    /// [`InvalidInput::WiderThanReferenced`].
    pub fn send_raw(&self, line: crate::Line) -> crate::RequestFuture<()> {
        let client = self.clone();
        crate::RequestFuture::new(async move { client.send_message(line.0).await })
    }

    /// Sends a CTCP reply privately to the user who sent `query`.
    ///
    /// The reply uses the query's command and CTCP framing even when the query was sent to a
    /// channel. It goes through the normal outbound queue and rate limit. Applications answering
    /// untrusted queries should also enforce a per-sender limit.
    ///
    /// An empty `text` sends the command without parameters, such as `\x01PING\x01`.
    /// A [`crate::event::CtcpReply`] cannot be passed to this method.
    /// Status-prefixed channel queries are answered privately to their actor; see
    /// [`crate::Conversation`] for the distinction between a conversation and a return address.
    /// Queries authored by this client's user are allowed so sibling devices can answer one
    /// another; the actor identity guard still refuses a reply after the nickname changes hands.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] when the command or text contains a CTCP or IRC line
    /// delimiter. It returns [`Error::TargetGone`] when the query sender can no longer be
    /// verified, a typed server refusal of the `NOTICE`, or [`Error::Disconnected`] if the
    /// driver stops before writing the reply.
    pub fn ctcp_reply(
        &self,
        query: &crate::event::CtcpQuery,
        text: impl Into<String>,
    ) -> crate::RequestFuture<SendOutcome> {
        let target = crate::target::sealed::Parts {
            send_name: None,
            text: std::borrow::Cow::Owned(query.actor.nick().to_string()),
            origin: crate::message_ref::conversation_origin(query.conversation()),
            channel: None,
            user: Some(query.actor.clone()),
        };
        let text = text.into();
        let command = query.command.clone();
        let body = Ctcp::build(
            command.as_str(),
            (!text.is_empty()).then_some(text.as_str()),
        );
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let body = body.ok_or(InvalidInput::InvalidCtcp)?;
            let target = client.resolve_send("ctcp_reply: query", target.into_owned())?;
            let state = client.state();
            let (status, conversation) = target.conversation(&client, &state)?;
            let receipt = state.sent_receipt(&conversation, target.text.clone());
            drop(state);
            let reply = Line::notice(&target.text, &body);
            let confirmation = client.send_one_line(target, reply).await?;
            let line = SentLine {
                text: body,
                confirmation,
            };
            Ok(SendOutcome::new(
                conversation,
                status,
                receipt,
                Vec::new(),
                vec![line],
            ))
        })
    }

    /// Sends one IRC protocol line and waits for its labelled response.
    ///
    /// Response lines are returned here rather than emitted by the event
    /// stream. Their state changes are still applied before this future
    /// completes. A message the server echoes (`PRIVMSG`, `NOTICE`, `TAGMSG`
    /// or `REDACT`) is answered only by its echo and its failure, so any other
    /// line in its reply, such as a service's answer, is also emitted as an event.
    ///
    /// Construct the [`crate::Line`] with [`crate::Line::new`] and [`crate::Line::param`], or
    /// explicitly parse text for a `/quote`-style input. Parsed parameters are ordinary text: an application that puts a
    /// credential in a parsed raw line chooses readable diagnostics and no secret-buffer wiping.
    /// Prefer a typed client method or pass [`crate::Secret`] to [`crate::Line::param`] for credentials.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when `labeled-response` is unavailable;
    /// raw requests have no unlabelled fallback. Also returns an error when the
    /// line cannot be represented on the wire, the request limit is reached,
    /// the response times out, or the driver stops.
    /// A send that widens a remembered status-prefixed line's audience returns
    /// [`InvalidInput::WiderThanReferenced`].
    pub fn request_raw(&self, line: crate::Line) -> crate::RequestFuture<Vec<crate::Line>> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let response = client.correlated(line.0, None).await?;
            Ok(response.lines.into_iter().map(crate::Line).collect())
        })
    }

    async fn send_message(&self, message: Line) -> Result<(), Error> {
        self.send_message_guarded(message, None).await
    }

    async fn send_message_guarded(
        &self,
        message: Line,
        guard: Option<papo_core::TargetGuard>,
    ) -> Result<(), Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::Send {
                message,
                guard,
                result,
            })
            .await
            .map_err(|_| Error::connection_lost())?;
        receiver.await.map_err(|_| Error::connection_lost())?
    }

    async fn send_one_line(
        &self,
        target: ResolvedSend,
        line: Line,
    ) -> Result<crate::Confirmation, Error> {
        if !self.state().supports(Feature::LabeledResponse) {
            self.send_chat_message(target, line).await?;
            return Ok(crate::Confirmation::unconfirmed(
                crate::UnconfirmedReason::NoLabeledResponse,
            ));
        }
        let response = self.correlated_chat(&target, line).await?;
        self.labelled_confirmation(&response)
    }

    pub(crate) fn labelled_confirmation(
        &self,
        response: &Response,
    ) -> Result<crate::Confirmation, Error> {
        crate::requests::empty(response)?;
        let echo_message = self.state().caps().contains_key(&crate::cap::ECHO_MESSAGE);
        Ok(crate::Confirmation::from_labelled_echo(
            echo_message,
            response.sent(),
        ))
    }

    fn redaction_confirmation(&self, response: &Response) -> Result<crate::Confirmation, Error> {
        crate::requests::empty(response)?;
        let echo = response
            .lines
            .iter()
            .map(Line::as_ref)
            .find(|line| line.command_is("REDACT"));
        let echo_message = self.state().caps().contains_key(&crate::cap::ECHO_MESSAGE);
        crate::Confirmation::from_redaction(echo_message, echo.as_ref())
    }

    async fn send_chat_message(&self, target: ResolvedSend, message: Line) -> Result<(), Error> {
        let Some(proof) = target.person else {
            return self.send_message_guarded(message, target.guard).await;
        };
        let command =
            papo_core::PersonCommand::__new(message).expect("a chat command has a person target");
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::SendPerson {
                proof,
                command,
                typing_target: target.text,
                result,
            })
            .await
            .map_err(|_| Error::connection_lost())?;
        receiver.await.map_err(|_| Error::connection_lost())?
    }

    pub(crate) async fn correlated(
        &self,
        message: Line,
        matcher: Option<ResponseMatcher>,
    ) -> Result<Response, Error> {
        self.correlated_guarded(message, matcher, None).await
    }

    pub(crate) async fn correlated_guarded(
        &self,
        message: Line,
        matcher: Option<ResponseMatcher>,
        guard: Option<papo_core::TargetGuard>,
    ) -> Result<Response, Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::Request {
                message,
                matcher,
                guard,
                result,
            })
            .await
            .map_err(|_| Error::connection_lost())?;
        receiver.await.map_err(|_| Error::connection_lost())?
    }

    async fn correlated_chat(
        &self,
        target: &ResolvedSend,
        message: Line,
    ) -> Result<Response, Error> {
        let Some(proof) = target.person.as_ref() else {
            return self
                .correlated_guarded(message, None, target.guard.clone())
                .await;
        };
        let command =
            papo_core::PersonCommand::__new(message).expect("a chat command has a person target");
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::RequestPerson {
                proof: proof.clone(),
                command,
                typing_target: target.text.clone(),
                result,
            })
            .await
            .map_err(|_| Error::connection_lost())?;
        receiver.await.map_err(|_| Error::connection_lost())?
    }

    pub(crate) async fn core_request(
        &self,
        request: impl Into<CoreRequest>,
    ) -> Result<Response, Error> {
        let request = request.into();
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::CoreRequest { request, result })
            .await
            .map_err(|_| Error::connection_lost())?;
        receiver.await.map_err(|_| Error::connection_lost())?
    }

    /// Sends a message to a channel, nickname, guarded [`crate::UserRef`], or return address.
    ///
    /// A nickname in text or as a [`crate::Nick`] addresses whoever holds that name.
    ///
    /// With `echo-message` and `labeled-response`, the future resolves with the server's copy,
    /// which is not also emitted as an event. Applications that log every message must log their
    /// own sends from the returned [`SendOutcome`]. Without both capabilities, it resolves after
    /// the last line is written, and every [`SentLine::confirmation`] says why it is unconfirmed.
    ///
    /// A conversation is deliberately not a send target because it does not retain a received
    /// line's status-prefixed audience:
    ///
    /// ```compile_fail
    /// # fn answer(client: &papo::Client, message: &papo::Message) {
    /// let _ = client.privmsg(&message.conversation, "acknowledged");
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] when name rules are unknown, the target is invalid, or
    /// the message is empty or unencodable. A guarded recipient that has left or whose nickname
    /// has changed hands returns [`Error::TargetGone`].
    /// [`Error::TooManyPendingRequests`] when 256 labelled requests are pending, [`Error::Timeout`] while
    /// awaiting a response, or [`Error::Disconnected`] when the driver stops. A refusal returns
    /// [`Error::Server`] with [`crate::Refusal::CannotSendToChannel`],
    /// [`crate::Refusal::NoSuchNick`], [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::InputTooLong`],
    /// [`crate::Refusal::TryAgain`], or [`crate::Refusal::Other`].
    pub fn privmsg(&self, target: impl crate::Target, text: impl Into<String>) -> SendMessage {
        SendMessage {
            client: self.clone(),
            kind: TextKind::Privmsg,
            target: crate::Target::__parts(&target).into_owned(),
            text: text.into(),
            reply_to: None,
            channel_context: None,
            tags: Vec::new(),
            action: false,
            invalid_input: None,
        }
    }

    /// Sends a notice to a channel, nickname, guarded direct recipient, or return address.
    ///
    /// Confirmation and event behavior is the same as [`Client::privmsg`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for empty text or an unencodable message,
    /// [`Error::TooManyPendingRequests`] when 256 labelled requests are pending, [`Error::Timeout`] while
    /// awaiting a response, or [`Error::Disconnected`] when the driver stops. A refusal returns
    /// [`Error::Server`] with [`crate::Refusal::CannotSendToChannel`],
    /// [`crate::Refusal::NoSuchNick`], [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::InputTooLong`],
    /// [`crate::Refusal::TryAgain`], or [`crate::Refusal::Other`].
    pub fn notice(&self, target: impl crate::Target, text: impl Into<String>) -> SendMessage {
        SendMessage {
            client: self.clone(),
            kind: TextKind::Notice,
            target: crate::Target::__parts(&target).into_owned(),
            text: text.into(),
            reply_to: None,
            channel_context: None,
            tags: Vec::new(),
            action: false,
            invalid_input: None,
        }
    }

    /// Sends a CTCP ACTION to a channel, nickname, guarded direct recipient, or return address.
    ///
    /// Confirmation and event behavior is the same as [`Client::privmsg`].
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Client::privmsg`].
    pub fn action(&self, target: impl crate::Target, text: impl Into<String>) -> SendMessage {
        let text = text.into();
        if Ctcp::action(&text).is_some() {
            let mut request = self.privmsg(target, text);
            request.action = true;
            request
        } else {
            let mut request = self.privmsg(target, text);
            request.invalid_input = Some(InvalidInput::InvalidCtcp);
            request
        }
    }

    /// Restores an opaque reference saved by [`crate::MessageRef::to_saved`] for this network.
    ///
    /// This is a local operation and writes nothing to the server. A private reference addresses
    /// whoever holds the saved nickname. Channel references retain the exact audience. See
    /// [`crate::Conversation`] for status-prefixed audience guidance.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidInput::NameRulesUnknown`] before the server's name rules are known, or
    /// [`InvalidInput::InvalidSavedMessage`] for invalid text, an unsupported version or
    /// address, or more than 10,000 identifiers or 1 MiB of text.
    pub fn saved_message(&self, text: &str) -> Result<crate::MessageRef, Error> {
        let state = self.state();
        state
            .known_name_rules()
            .ok_or(InvalidInput::NameRulesUnknown)?;
        let (address, ids) =
            crate::message_ref::saved::parse(text).ok_or(InvalidInput::InvalidSavedMessage)?;
        let (status, target) = state
            .parse_target_inner(&address)
            .map_err(|_| InvalidInput::InvalidSavedMessage)?;
        if status.is_some() {
            refuse_stacked_status(&state, &address)
                .map_err(|_| InvalidInput::InvalidSavedMessage)?;
        }
        let private = matches!(target, papo_core::Conversation::Private(_));
        Ok(crate::MessageRef::restored(
            address,
            ids,
            self.origin,
            private,
        ))
    }

    /// Sends a private message threaded as a reply to `message` when supported.
    ///
    /// Passing the received line or [`SendOutcome`] preserves its return address. See
    /// [`crate::Conversation`] for status-prefixed line guidance.
    /// A notice is not a reply input:
    ///
    /// ```compile_fail
    /// # fn notice(client: &papo::Client, notice: &papo::event::Notice) {
    /// let _ = client.reply(notice, "automatic response");
    /// # }
    /// ```
    ///
    /// A live private return address follows observed nickname changes. It expires on
    /// reconnect or when papo observes the sender leave or the nickname change hands.
    /// A [`crate::MessageRef`] from private history addresses the thread's peer by nickname
    /// and refuses your own nickname.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TargetGone`] in the private-recipient case described above. It returns an
    /// error if the text is empty or invalid, or the driver has stopped.
    /// A send that widens a remembered status-prefixed line's audience returns
    /// [`InvalidInput::WiderThanReferenced`].
    pub fn reply(
        &self,
        message: impl Into<crate::MessageRef>,
        text: impl Into<String>,
    ) -> SendMessage {
        let message = message.into();
        let msgid = message.msgid().cloned();
        let channel_context = message.channel_context().cloned();
        let mut request = self.privmsg(message, text);
        if let Some(id) = msgid {
            request = request.reply_to(id);
        }
        if let Some(context) = channel_context {
            request = request.channel_context(context);
        }
        request
    }

    /// Adds `emoji` as a reaction to `message`.
    ///
    /// A reaction is usually an emoji but may be any nonempty text. Reactions are compared
    /// exactly, without case folding or Unicode normalization.
    /// The result carries the server's msgid and time for the reaction when `echo-message` and
    /// `labeled-response` let it confirm one; see [`ReactOutcome`].
    /// Passing the received line or [`SendOutcome`] preserves its return address; see
    /// [`crate::Conversation`] for status-prefixed line guidance.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`Feature::Reactions`] with reason
    /// [`UnsupportedReason::NotOffered`], [`UnsupportedReason::DisabledByClient`], or
    /// [`UnsupportedReason::TagDenied`]. Unknown name rules, an invalid target, or encoding
    /// failure returns [`Error::InvalidInput`]. A driver failure returns [`Error::Disconnected`]. A reference without a server message
    /// identifier returns [`InvalidInput::UnusableMessageReference`].
    /// A private reference whose nickname is proven to belong to another user returns
    /// [`Error::TargetGone`].
    /// A send that widens a remembered status-prefixed line's audience returns
    /// [`InvalidInput::WiderThanReferenced`].
    pub fn react(
        &self,
        message: impl Into<crate::MessageRef>,
        emoji: impl Into<String>,
    ) -> crate::RequestFuture<ReactOutcome> {
        self.reaction(message.into(), emoji.into(), "+draft/react")
    }

    /// Removes `emoji` as a reaction to `message`.
    ///
    /// `emoji` must exactly match the added reaction; papo performs no Unicode normalization.
    /// The result is confirmed the same way as [`Client::react`].
    /// Passing the received line or [`SendOutcome`] preserves its return address; see
    /// [`crate::Conversation`] for status-prefixed line guidance.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`Feature::Reactions`] with reason
    /// [`UnsupportedReason::NotOffered`], [`UnsupportedReason::DisabledByClient`], or
    /// [`UnsupportedReason::TagDenied`]. Encoding or driver failure returns
    /// [`Error::InvalidInput`] or [`Error::Disconnected`]. A reference without a server message
    /// identifier returns [`InvalidInput::UnusableMessageReference`].
    /// A private reference whose nickname is proven to belong to another user returns
    /// [`Error::TargetGone`].
    /// A send that widens a remembered status-prefixed line's audience returns
    /// [`InvalidInput::WiderThanReferenced`].
    pub fn unreact(
        &self,
        message: impl Into<crate::MessageRef>,
        emoji: impl Into<String>,
    ) -> crate::RequestFuture<ReactOutcome> {
        self.reaction(message.into(), emoji.into(), "+draft/unreact")
    }

    fn reaction(
        &self,
        message: crate::MessageRef,
        emoji: String,
        reaction_tag: &'static str,
    ) -> crate::RequestFuture<ReactOutcome> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let Some(message_id) = message.msgid().cloned() else {
                return Err(InvalidInput::UnusableMessageReference.into());
            };
            if emoji.is_empty() {
                return Err(InvalidInput::EmptyText.into());
            }
            if emoji.len() > papo_proto::MAX_CLIENT_TAG_DATA_LEN {
                return Err(InvalidInput::Reaction {
                    maximum: papo_proto::MAX_CLIENT_TAG_DATA_LEN,
                    actual: emoji.len(),
                }
                .into());
            }
            let target = client.resolve_send(
                "reaction: message",
                crate::Target::__parts(&message).into_owned(),
            )?;
            let state = client.state();
            Self::require_feature(&state, Feature::Reactions)?;
            message_id
                .check()
                .map_err(|error| InvalidInput::invalid_name(message_id.to_string(), error))?;
            let denied = state.isupport().clienttagdeny();
            // Feature preflight rejects the only state where both aliases are unavailable.
            let reply_tag = event_impl::allowed_alias(&denied, "+reply", "+draft/reply")
                .expect("feature preflight requires an allowed reply tag");
            if denied.denies(reaction_tag) {
                return Err(Error::Unsupported(Unsupported::new(
                    Feature::Reactions,
                    UnsupportedReason::TagDenied,
                )));
            }
            let line = Line::tagmsg(&target.text)
                .with_tag(reply_tag, message_id.as_ref())
                .with_tag(reaction_tag, emoji);
            let (status, conversation) = target.conversation(&client, &state)?;
            let confirmation = client.send_one_line(target, line).await?;
            Ok(ReactOutcome {
                conversation,
                status,
                confirmation,
            })
        })
    }

    /// Sends a typing state to a channel, nickname, guarded direct recipient, or return address.
    ///
    /// A nickname in text or as a [`crate::Nick`] addresses whoever holds that name.
    ///
    /// While composing, send [`Typing::Active`] every three seconds so recipients can refresh
    /// their local expiry deadline. Repeated active states within three seconds are silently
    /// dropped.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`Feature::Typing`] with reason
    /// [`UnsupportedReason::NotOffered`], [`UnsupportedReason::DisabledByClient`], or
    /// [`UnsupportedReason::TagDenied`]. Encoding or driver failure returns
    /// [`Error::InvalidInput`] or [`Error::Disconnected`].
    pub fn set_typing(
        &self,
        target: impl crate::Target,
        state: Typing,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let target = crate::Target::__parts(&target).into_owned();
        crate::RequestFuture::new(async move {
            let target = client.resolve_send("typing: target", target)?;
            let snapshot = client.state();
            Self::require_feature(&snapshot, Feature::Typing)?;
            let (result, receiver) = oneshot::channel();
            client
                .commands
                .send(Command::Typing {
                    target: target.text.clone(),
                    state,
                    guard: target.into_chat_guard(),
                    result,
                })
                .await
                .map_err(|_| Error::connection_lost())?;
            receiver.await.map_err(|_| Error::connection_lost())?
        })
    }

    /// Redacts a message from a conversation's history.
    ///
    /// Passing the received line or [`SendOutcome`] preserves its return address. See
    /// [`crate::Conversation`] for status-prefixed line guidance.
    /// A sent message split across multiple lines is redacted in send order. Redaction stops at
    /// the first error, and any earlier lines remain redacted.
    /// An empty `reason` means no reason.
    ///
    /// Resolves to a [`RedactOutcome`]. Each line's confirmation comes from our `REDACT` echo,
    /// which Ergo sends outside the labelled response, just before its `ACK`. With
    /// `echo-message`, an `ACK` without that echo returns [`Error::NotVerified`]: the server
    /// accepted the request but did not prove the redaction. Without `echo-message`, it is
    /// [`crate::Confirmation::Unconfirmed`] with [`crate::UnconfirmedReason::NoEchoMessage`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`Feature::Redaction`] with reason
    /// [`UnsupportedReason::NotOffered`] or [`UnsupportedReason::DisabledByClient`]. A refusal
    /// returns [`Error::Server`], currently classified as [`crate::Refusal::Other`]. It can also
    /// return [`Error::NotVerified`], [`Error::InvalidInput`], [`Error::TooManyPendingRequests`],
    /// [`Error::Timeout`], or [`Error::Disconnected`]. A reference without a server message identifier returns
    /// [`InvalidInput::UnusableMessageReference`].
    /// A private reference whose nickname is proven to belong to another user returns
    /// [`Error::TargetGone`].
    /// A send that widens a remembered status-prefixed line's audience returns
    /// [`InvalidInput::WiderThanReferenced`].
    pub fn redact(
        &self,
        message: impl Into<crate::MessageRef>,
        reason: impl Into<String>,
    ) -> crate::RequestFuture<RedactOutcome> {
        let client = self.clone();
        let message = message.into();
        let reason = reason.into();
        crate::RequestFuture::new(async move {
            if message.msgids().is_empty() {
                return Err(InvalidInput::UnusableMessageReference.into());
            }
            let target = client.resolve_send(
                "redact: message",
                crate::Target::__parts(&message).into_owned(),
            )?;
            let state = client.state();
            Self::require_feature(&state, Feature::Redaction)?;
            let (status, conversation) = target.conversation(&client, &state)?;
            drop(state);
            let commands = client.commands.clone();
            let (result, receiver) = oneshot::channel();
            let task = Box::pin(async move {
                let outcome = async {
                    let mut lines = Vec::with_capacity(message.msgids().len());
                    for message_id in message.msgids() {
                        message_id.check().map_err(|error| {
                            InvalidInput::invalid_name(message_id.to_string(), error)
                        })?;
                        let mut params = vec![target.text.clone(), message_id.to_string()];
                        if !reason.is_empty() {
                            params.push(reason.clone());
                        }
                        let response = client
                            .correlated_chat(&target, Line::new("REDACT").with_params(params))
                            .await?;
                        lines.push(RedactedLine {
                            redacts: message_id.clone(),
                            confirmation: client.redaction_confirmation(&response)?,
                        });
                    }
                    Ok(RedactOutcome {
                        conversation,
                        status,
                        lines,
                    })
                }
                .await;
                _ = result.send(outcome);
            });
            commands
                .send(Command::Composite { task })
                .await
                .map_err(|_| Error::connection_lost())?;
            receiver.await.map_err(|_| Error::connection_lost())?
        })
    }

    async fn send_text(
        &self,
        kind: TextKind,
        target: ResolvedSend,
        text: &str,
        tags: Vec<(String, String)>,
        fallbacks: Vec<Fallback>,
        action: bool,
    ) -> Result<SendOutcome, Error> {
        let state = self.state();
        let wire_target = target.text.clone();
        let (status, conversation) = target.conversation(self, &state)?;
        let receipt = state.sent_receipt(&conversation, wire_target.clone());
        if !state.is_connected() {
            let text = if action {
                Ctcp::action(text).ok_or(InvalidInput::InvalidCtcp)?
            } else {
                text.to_owned()
            };
            let mut message = match kind {
                TextKind::Privmsg => Line::privmsg(&wire_target, &text),
                TextKind::Notice => Line::notice(&wire_target, &text),
            };
            for (key, value) in tags {
                message = message.with_tag(key, value);
            }
            self.send_chat_message(target, message).await?;
            let lines =
                SentLine::unconfirmed(vec![text], crate::UnconfirmedReason::NoLabeledResponse);
            return Ok(SendOutcome::new(
                conversation,
                status,
                receipt,
                fallbacks,
                lines,
            ));
        }
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::SendText {
                kind,
                target: wire_target,
                conversation,
                status,
                receipt,
                text: text.to_owned(),
                tags,
                fallbacks,
                action,
                guard: target.into_chat_guard(),
                result,
            })
            .await
            .map_err(|_| Error::connection_lost())?;
        receiver.await.map_err(|_| Error::connection_lost())?
    }

    /// Requests information about a nickname.
    ///
    /// # Errors
    ///
    /// Returns `None` when the target is not online. Returns [`Error::Server`] with
    /// [`crate::Refusal::NoSuchServer`], [`crate::Refusal::TryAgain`], or
    /// [`crate::Refusal::Other`] when refused. A reply longer than 10,000 lines returns
    /// [`Error::StateLimit`]. It can also return [`Error::InvalidInput`],
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    /// ```compile_fail
    /// # fn swapped(client: &papo::Client, channel: &papo::ChannelName) {
    /// let _ = client.whois(channel);
    /// # }
    /// ```
    pub fn whois(&self, nick: impl crate::AsNick) -> crate::RequestFuture<Option<crate::Whois>> {
        let client = self.clone();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        crate::RequestFuture::new(async move {
            let proven = crate::identity::prove_parts(&client, nick, "whois: nick")?;
            let nick = proven.nick;
            let response = client
                .correlated_guarded(
                    Line::new("WHOIS").with_params([nick.to_string()]),
                    Some(ResponseMatcher::Whois(nick.to_string())),
                    Some(papo_core::TargetGuard::Proven(proven.core)),
                )
                .await?;
            crate::requests::whois(
                response,
                nick.as_str(),
                client.state().isupport().inner(),
                &client,
            )
        })
    }

    /// Requests the users matching a channel or mask.
    ///
    /// When the argument names a joined channel, the reply refreshes that channel's member facts
    /// in [`crate::State`] and emits [`crate::Event::MembersRefreshed`] at numeric `315`.
    /// Channel refreshes and single-nickname queries emit [`crate::Event::AwayChanged`] for
    /// changes to a known away status.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::TryAgain`] or
    /// [`crate::Refusal::Other`] when refused. A reply longer than 10,000 lines returns
    /// [`Error::StateLimit`] without a partial list. It can also return [`Error::InvalidInput`],
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub fn who(&self, mask: impl crate::AsMask) -> crate::RequestFuture<Vec<crate::WhoEntry>> {
        let client = self.clone();
        let mask = crate::AsMask::__parts(&mask).into_owned();
        crate::RequestFuture::new(async move {
            let mask = client.resolve(crate::target::Role::Mask("who: mask"), mask)?;
            let mask = client
                .state()
                .known_name_rules()
                .ok_or(InvalidInput::NameRulesUnknown)?
                .mask(mask);
            let state = client.state();
            let extended = state.isupport().whox();
            let message = if extended {
                Line::new("WHO")
                    .with_params([mask.to_string(), crate::requests::WHOX_FIELDS.into()])
            } else {
                Line::new("WHO").with_params([mask.to_string()])
            };
            let response = client
                .correlated(message, Some(ResponseMatcher::Who(mask.to_string())))
                .await?;
            crate::requests::who(response, state.isupport().inner(), &client, extended)
        })
    }

    /// Joins a channel and returns its channel reference after the topic and member list synchronize.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::TooManyChannels`], [`crate::Refusal::TemporarilyUnavailable`],
    /// [`crate::Refusal::ChannelIsFull`], [`crate::Refusal::InviteOnlyChannel`],
    /// [`crate::Refusal::BannedFromChannel`], [`crate::Refusal::BadChannelKey`],
    /// [`crate::Refusal::BadChannelMask`], [`crate::Refusal::SecureOnlyChannel`],
    /// [`crate::Refusal::TryAgain`], or [`crate::Refusal::Other`] when refused. It can also
    /// return [`Error::StateLimit`] before sending when a new channel would exceed the configured
    /// state bound, [`Error::InvalidInput`], [`Error::TooManyPendingRequests`], [`Error::Timeout`],
    /// or [`Error::Disconnected`].
    /// ```compile_fail
    /// # fn swapped(client: &papo::Client, nick: &papo::Nick) {
    /// let _ = client.join(nick);
    /// # }
    /// ```
    pub fn join(&self, channel: impl crate::AsChannel) -> JoinChannel {
        JoinChannel {
            client: self.clone(),
            channel: crate::AsChannel::__parts(&channel).into_owned(),
            key: None,
        }
    }

    async fn join_channel(
        &self,
        channel: crate::ChannelName,
        key: Option<crate::Secret>,
    ) -> Result<crate::ChannelRef, Error> {
        channel
            .check(self.state().isupport().inner())
            .map_err(|reason| InvalidInput::invalid_name(channel.to_string(), reason))?;
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::Join {
                channel: channel.clone(),
                key,
                result,
            })
            .await
            .map_err(|_| Error::connection_lost())?;
        let response = receiver.await.map_err(|_| Error::connection_lost())??;
        if let Some(error) = response.error() {
            return Err(Error::server(error));
        }
        if self.state().channel_local(&channel).is_none() {
            return Err(Error::unexpected_reply("JOIN"));
        }
        if !self
            .state()
            .channel_local(&channel)
            .is_some_and(crate::Channel::is_synced)
        {
            let (result, receiver) = oneshot::channel();
            self.commands
                .send(Command::WaitChannelSync {
                    channel: channel.to_string(),
                    result,
                })
                .await
                .map_err(|_| Error::connection_lost())?;
            receiver.await.map_err(|_| Error::connection_lost())??;
        }
        Ok(crate::ChannelRef::new(self, channel, true))
    }

    /// Leaves a channel and resolves to the server's echo of our `PART`.
    ///
    /// Add a reason with [`PartRequest::reason`]:
    ///
    /// ```no_run
    /// # async fn leave(client: &papo::Client) -> Result<(), papo::Error> {
    /// client.part("#papo").await?;
    /// client.part("#rust").reason("Back tomorrow").await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// While disconnected, this removes the channel from the automatic rejoin set and returns
    /// [`Error::Disconnected`] without writing a `PART`. If a bouncer restores the channel after
    /// reconnection, papo sends `PART` then.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::NotOnChannel`], [`crate::Refusal::TryAgain`], or
    /// [`crate::Refusal::Other`] when refused, and [`Error::NotVerified`] when answered without
    /// the echo. It can also return [`Error::InvalidInput`], [`Error::TooManyPendingRequests`],
    /// [`Error::Timeout`], or [`Error::Disconnected`].
    pub fn part(&self, channel: impl crate::AsChannel) -> PartRequest {
        PartRequest {
            client: self.clone(),
            channel: crate::AsChannel::__parts(&channel).into_owned(),
            reason: String::new(),
        }
    }

    fn part_request(
        &self,
        channel: crate::target::sealed::Parts<'static>,
        reason: String,
    ) -> crate::RequestFuture<crate::PartOutcome> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let channel = client.resolve(crate::target::Role::Channel("part: channel"), channel)?;
            let channel = client
                .state()
                .known_name_rules()
                .ok_or(InvalidInput::NameRulesUnknown)?
                .channel(channel);
            let (result, receiver) = oneshot::channel();
            client
                .commands
                .send(Command::Part {
                    channel,
                    reason,
                    result,
                })
                .await
                .map_err(|_| Error::connection_lost())?;
            let response = receiver.await.map_err(|_| Error::connection_lost())??;
            let echo = crate::requests::echo(&response, "PART")?;
            Ok(crate::PartOutcome {
                confirmation: crate::Confirmation::from_echo(&echo),
            })
        })
    }

    /// Removes `nick` from `channel` and resolves to the server's echo of our `KICK`.
    ///
    /// A [`crate::UserRef`] follows its proven identity; text or [`crate::Nick`] addresses
    /// whoever currently holds that nickname. Add a reason with [`KickRequest::reason`]:
    ///
    /// ```no_run
    /// # async fn remove(client: &papo::Client) -> Result<(), papo::Error> {
    /// client.kick("#papo", "spambot").await?;
    /// client.kick("#papo", "mallory").reason("Flooding").await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ```compile_fail
    /// # fn swapped(
    /// #     client: &papo::Client,
    /// #     nick: &papo::Nick,
    /// #     channel: &papo::ChannelName,
    /// # ) {
    /// let _ = client.kick(nick, channel);
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotVerified`] when the server answers without the echo, or a typed server
    /// refusal, invalid-input error, timeout, or disconnect.
    pub fn kick(&self, channel: impl crate::AsChannel, nick: impl crate::AsNick) -> KickRequest {
        KickRequest {
            client: self.clone(),
            channel: crate::AsChannel::__parts(&channel).into_owned(),
            nick: crate::AsNick::__parts(&nick).into_owned(),
            reason: String::new(),
        }
    }

    fn kick_request(
        &self,
        channel: crate::target::sealed::Parts<'static>,
        nick: crate::target::sealed::Parts<'static>,
        reason: String,
    ) -> crate::RequestFuture<crate::KickOutcome> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let channel = client.resolve(crate::target::Role::Channel("kick: channel"), channel)?;
            let state = client.state();
            let channel = state
                .known_name_rules()
                .ok_or(InvalidInput::NameRulesUnknown)?
                .channel(channel);
            drop(state);
            kick_resolved(&client, channel, nick, reason).await
        })
    }

    /// Sets a channel ban and, only after confirming it, removes `nick` from the channel.
    ///
    /// Name the nickname with [`Kickban::nick`], then the ban with [`KickbanNick::ban`]; only the
    /// resulting [`KickbanRequest`] can be awaited.
    ///
    /// A [`crate::UserRef`] is checked again after ban confirmation. Text or [`crate::Nick`]
    /// deliberately addresses whoever currently holds that nickname.
    /// The ban remains set when the kick is refused. Leaving or changing nickname during ban
    /// confirmation produces [`Error::TargetGone`] in [`crate::KickbanOutcome::kick`].
    /// An empty `reason` means no reason.
    ///
    /// ```compile_fail
    /// # fn mask_is_not_a_reason(client: &papo::Client, reason: &papo::Mask) {
    /// let _ = client.kickban("#papo").nick("mallory").ban("*!*@gateway.example").reason(reason);
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotVerified`] without sending `KICK` when the ban cannot be proved. A ban
    /// refusal and the errors documented by [`Client::kick`] are returned before an outcome.
    pub fn kickban(&self, channel: impl crate::AsChannel) -> Kickban {
        Kickban {
            client: self.clone(),
            channel: crate::AsChannel::__parts(&channel).into_owned(),
        }
    }

    fn kickban_request(
        &self,
        channel: crate::target::sealed::Parts<'static>,
        nick: crate::target::sealed::Parts<'static>,
        ban: crate::BanTarget,
        reason: String,
    ) -> crate::RequestFuture<crate::KickbanOutcome> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let channel =
                client.resolve(crate::target::Role::Channel("kickban: channel"), channel)?;
            let resolved_nick =
                client.resolve(crate::target::Role::Nick("kickban: nick"), nick.clone())?;
            let state = client.state();
            let names = state
                .known_name_rules()
                .ok_or(InvalidInput::NameRulesUnknown)?;
            let channel = names.channel(channel);
            let resolved_nick = names.nick(resolved_nick);
            validate_kick_reason(&state, &reason)?;
            let ban = ban.resolve(&state)?;
            let already_set = crate::mode::tracked_ban(&state, &channel, &ban);
            let target_was_present = state
                .channel_local(&channel)
                .is_some_and(|joined| joined.member(&resolved_nick).unwrap().is_some());
            drop(state);
            let commands = client.commands.clone();
            let (result, receiver) = oneshot::channel();
            let task = Box::pin(async move {
                let outcome = async {
                    let ban = if let Some(stored) = already_set {
                        stored
                    } else {
                        let stored = set_ban(&client, &channel, &ban).await?;
                        if target_was_present
                            && client.state().channel_local(&channel).is_none_or(|joined| {
                                joined.member(&resolved_nick).unwrap().is_none()
                            })
                        {
                            return Ok(crate::KickbanOutcome {
                                ban: stored,
                                kick: Err(Error::TargetGone),
                            });
                        }
                        stored
                    };
                    Ok(crate::KickbanOutcome {
                        kick: kick_resolved(&client, channel, nick, reason).await,
                        ban,
                    })
                }
                .await;
                _ = result.send(outcome);
            });
            commands
                .send(Command::Composite { task })
                .await
                .map_err(|_| Error::connection_lost())?;
            receiver.await.map_err(|_| Error::connection_lost())?
        })
    }

    /// Invites `nick` to `channel`.
    ///
    /// A [`crate::UserRef`] retains the user's identity. Text and [`crate::Nick`] address the
    /// current holder.
    ///
    /// # Errors
    ///
    /// Returns a typed server refusal, invalid-input error, timeout, or disconnect.
    pub fn invite(
        &self,
        nick: impl crate::AsNick,
        channel: impl crate::AsChannel,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let channel = crate::AsChannel::__parts(&channel).into_owned();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        crate::RequestFuture::new(async move {
            let channel =
                client.resolve(crate::target::Role::Channel("invite: channel"), channel)?;
            let proven = crate::identity::prove_parts(&client, nick, "invite: nick")?;
            let state = client.state();
            let names = state
                .known_name_rules()
                .ok_or(InvalidInput::NameRulesUnknown)?;
            let channel = names.channel(channel);
            let nick = proven.nick;
            let response = client
                .correlated_guarded(
                    Line::new("INVITE").with_params([nick.to_string(), channel.to_string()]),
                    Some(ResponseMatcher::__invite(
                        channel.to_string(),
                        nick.to_string(),
                    )),
                    Some(papo_core::TargetGuard::Proven(proven.core)),
                )
                .await?;
            crate::requests::empty(&response)
        })
    }

    /// Changes this client's nickname and resolves to the server's `NICK` echo.
    ///
    /// An empty nickname is invalid input.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotVerified`] when the server answers without the echo, as Ergo does for
    /// the current nickname, or a typed server refusal, invalid-input error, timeout, or
    /// disconnect.
    /// ```compile_fail
    /// # fn swapped(client: &papo::Client, channel: &papo::ChannelName) {
    /// let _ = client.set_nick(channel);
    /// # }
    /// ```
    pub fn set_nick(&self, nick: impl crate::AsNick) -> crate::RequestFuture<crate::NickOutcome> {
        let client = self.clone();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        crate::RequestFuture::new(async move {
            let nick = client.resolve(crate::target::Role::Nick("set_nick: nick"), nick)?;
            let state = client.state();
            let names = state
                .known_name_rules()
                .ok_or(InvalidInput::NameRulesUnknown)?;
            let nick = names.nick(nick);
            let response = client
                .correlated(
                    Line::new("NICK").with_params([nick.to_string()]),
                    Some(ResponseMatcher::Nick(nick.to_string())),
                )
                .await?;
            let echo = crate::requests::echo(&response, "NICK")?;
            Ok(crate::NickOutcome {
                nick: echo.param(0).map_or(nick, |echoed| names.nick(echoed)),
                confirmation: crate::Confirmation::from_echo(&echo),
            })
        })
    }

    /// Marks this client away with a nonempty message.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for an empty message, or a typed server refusal,
    /// timeout, or disconnect.
    pub fn set_away(&self, text: impl Into<String>) -> crate::RequestFuture<()> {
        let client = self.clone();
        let text = text.into();
        crate::RequestFuture::new(async move {
            if text.is_empty() {
                return Err(InvalidInput::EmptyText.into());
            }
            let response = client
                .correlated(
                    Line::new("AWAY").with_params([text]),
                    Some(ResponseMatcher::Away(true)),
                )
                .await?;
            crate::requests::empty(&response)
        })
    }

    /// Clears this client's away status.
    ///
    /// # Errors
    ///
    /// Returns a typed server refusal, timeout, or disconnect.
    pub fn clear_away(&self) -> crate::RequestFuture<()> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let response = client
                .correlated(Line::new("AWAY"), Some(ResponseMatcher::Away(false)))
                .await?;
            crate::requests::empty(&response)
        })
    }

    /// Changes this client's realname through the `setname` capability and resolves to the
    /// realname the server echoed.
    ///
    /// An empty realname is invalid input.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when `setname` is unavailable, [`Error::NotVerified`] when
    /// the server answers without the echo, or a typed server refusal, invalid-input error,
    /// timeout, or disconnect.
    pub fn set_realname(
        &self,
        realname: impl Into<String>,
    ) -> crate::RequestFuture<crate::RealnameOutcome> {
        let client = self.clone();
        let realname = realname.into();
        crate::RequestFuture::new(async move {
            if realname.is_empty() {
                return Err(InvalidInput::EmptyText.into());
            }
            Self::require_feature(&client.state(), Feature::SetName)?;
            let response = client
                .correlated(
                    Line::new("SETNAME").with_params([realname]),
                    Some(ResponseMatcher::SetName),
                )
                .await?;
            let echo = crate::requests::echo(&response, "SETNAME")?;
            Ok(crate::RealnameOutcome {
                realname: echo.param(0).ok_or(Error::NotVerified)?.to_owned(),
                confirmation: crate::Confirmation::from_echo(&echo),
            })
        })
    }

    /// Authenticates as an IRC operator.
    ///
    /// # Errors
    ///
    /// Returns a typed server refusal, invalid-input error, timeout, or disconnect.
    pub fn oper(
        &self,
        name: impl Into<String>,
        password: crate::Secret,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let name = name.into();
        crate::RequestFuture::new(async move {
            let response = client
                .core_request(CoreRequest::Oper { name, password })
                .await?;
            crate::requests::empty(&response)
        })
    }

    /// Measures a server round trip using `PING` and its matching `PONG`.
    ///
    /// # Errors
    ///
    /// Returns a typed server refusal, timeout, or disconnect.
    pub fn ping(&self) -> crate::RequestFuture<Duration> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let token = format!("papo-lag-{}", NEXT_PING.fetch_add(1, Ordering::Relaxed));
            let started = crate::rt::now_mono();
            let response = client
                .correlated(
                    Line::new("PING").with_params([token.clone()]),
                    Some(ResponseMatcher::Ping(token)),
                )
                .await?;
            crate::requests::empty(&response)?;
            Ok(crate::rt::now_mono().saturating_duration_since(started))
        })
    }

    /// Sets the topic for `channel` and resolves to the topic the server echoed.
    ///
    /// An empty `topic` is invalid input. Use [`Client::clear_topic`] to clear it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotVerified`] when the server answers without the echo, or
    /// [`Error::Server`] with [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::NotOnChannel`], [`crate::Refusal::ChannelOperatorPrivilegesNeeded`],
    /// [`crate::Refusal::TryAgain`], or [`crate::Refusal::Other`] when refused. It can also
    /// return [`Error::InvalidInput`], [`Error::TooManyPendingRequests`], [`Error::Timeout`], or
    /// [`Error::Disconnected`].
    pub fn set_topic(
        &self,
        channel: impl crate::AsChannel,
        topic: impl Into<String>,
    ) -> crate::RequestFuture<crate::TopicOutcome> {
        self.change_topic(
            crate::AsChannel::__parts(&channel).into_owned(),
            topic.into(),
            false,
        )
    }

    /// Clears the topic for `channel`.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Client::set_topic`].
    pub fn clear_topic(
        &self,
        channel: impl crate::AsChannel,
    ) -> crate::RequestFuture<crate::TopicOutcome> {
        self.change_topic(
            crate::AsChannel::__parts(&channel).into_owned(),
            String::new(),
            true,
        )
    }

    fn change_topic(
        &self,
        channel: crate::target::sealed::Parts<'static>,
        topic: String,
        clear: bool,
    ) -> crate::RequestFuture<crate::TopicOutcome> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let channel =
                client.resolve(crate::target::Role::Channel("topic: channel"), channel)?;
            let channel = client
                .state()
                .known_name_rules()
                .ok_or(InvalidInput::NameRulesUnknown)?
                .channel(channel);
            if topic.is_empty() && !clear {
                return Err(InvalidInput::EmptyText.into());
            }
            let response = client
                .correlated(
                    Line::new("TOPIC").with_params([channel.to_string(), topic]),
                    Some(ResponseMatcher::Topic(channel.to_string())),
                )
                .await?;
            let echo = crate::requests::echo(&response, "TOPIC")?;
            Ok(crate::TopicOutcome {
                topic: echo.param(1).ok_or(Error::NotVerified)?.to_owned(),
                confirmation: crate::Confirmation::from_echo(&echo),
            })
        })
    }

    /// Starts a request for visible channels.
    ///
    /// At most 10,000 entries are retained.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::TryAgain`] or
    /// [`crate::Refusal::Other`] when refused. A longer reply returns [`Error::StateLimit`]
    /// without a partial list. It can also return [`Error::InvalidInput`],
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub fn list(&self) -> crate::ListRequest {
        crate::ListRequest::new(self.clone())
    }

    /// Requests a clean disconnect and waits for the peer to close.
    ///
    /// The wait is limited to ten seconds. The driver still completes cleanly
    /// when that limit is reached. An I/O failure while waiting for closure
    /// also resolves to `Ok(())`. A timed-out acknowledgement is recorded in
    /// the final [`crate::Event::Disconnected`]. Add a reason with [`QuitRequest::reason`]:
    ///
    /// ```no_run
    /// # async fn leave(client: &papo::Client) -> Result<(), papo::Error> {
    /// client.quit().reason("Upgrading").await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error only if the QUIT message is invalid.
    pub fn quit(&self) -> QuitRequest {
        QuitRequest {
            client: self.clone(),
            reason: String::new(),
        }
    }

    fn quit_request(&self, reason: String) -> crate::RequestFuture<()> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let message = if reason.is_empty() {
                Line::new("QUIT")
            } else {
                Line::new("QUIT").with_params([reason])
            };
            message.to_line().map_err(Error::build)?;
            let (result, receiver) = oneshot::channel();
            if client
                .commands
                .send(Command::Quit { message, result })
                .await
                .is_err()
            {
                return Ok(());
            }
            receiver.await.unwrap_or(Ok(()))
        })
    }
}

async fn set_ban(
    client: &Client,
    channel: &crate::ChannelName,
    ban: &crate::Mask,
) -> Result<crate::Mask, Error> {
    let response = client
        .correlated(
            Line::new("MODE").with_params([channel.as_str(), "+b", ban.as_str()]),
            Some(ResponseMatcher::Mode(channel.to_string())),
        )
        .await?;
    let already_set_reply = response
        .error()
        .is_some_and(|error| error.refusal() == crate::Refusal::ListModeAlreadySet);
    if let Some(error) = response.error()
        && !already_set_reply
    {
        return Err(Error::server(error));
    }
    let state = client.state();
    let stored = if already_set_reply {
        Some(crate::mode::tracked_ban(&state, channel, ban).unwrap_or_else(|| ban.clone()))
    } else {
        crate::mode::confirmed_ban(&response.lines, &state, channel, ban)
            .or_else(|| crate::mode::tracked_ban(&state, channel, ban))
    };
    if let Some(stored) = stored {
        return Ok(stored);
    }
    let list_known = state
        .channel_local(channel)
        .is_some_and(|joined| joined.list(crate::ListMode::Ban).is_some());
    drop(state);
    if list_known {
        return Err(Error::NotVerified);
    }
    let entries = client.list_modes(channel, crate::ListMode::Ban).await?;
    let state = client.state();
    entries
        .into_iter()
        .find(|entry| state.same_list_entry(&entry.mask, ban.as_str()))
        .map(|entry| entry.mask)
        .ok_or(Error::NotVerified)
}

async fn kick_resolved(
    client: &Client,
    channel: crate::ChannelName,
    nick: crate::target::sealed::Parts<'static>,
    reason: String,
) -> Result<crate::KickOutcome, Error> {
    let proven = crate::identity::prove_parts(client, nick, "kick: nick")?;
    let state = client.state();
    let nick = proven.nick;
    validate_kick_reason(&state, &reason)?;
    let mut params = vec![channel.to_string(), nick.to_string()];
    if !reason.is_empty() {
        params.push(reason);
    }
    let response = client
        .correlated_guarded(
            Line::new("KICK").with_params(params),
            Some(ResponseMatcher::__kick(
                channel.to_string(),
                nick.to_string(),
            )),
            Some(papo_core::TargetGuard::ProvenAt(proven.core, 1)),
        )
        .await?;
    let echo = crate::requests::echo(&response, "KICK")?;
    Ok(crate::KickOutcome {
        confirmation: crate::Confirmation::from_echo(&echo),
    })
}

pub(crate) fn validate_kick_reason(state: &State, reason: &str) -> Result<(), Error> {
    if let Some(maximum) = state.isupport().kicklen()
        && reason.len() > maximum
    {
        return Err(InvalidInput::KickReasonTooLong {
            maximum,
            actual: reason.len(),
        }
        .into());
    }
    Ok(())
}

impl std::fmt::Debug for Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("Client", formatter)
    }
}
