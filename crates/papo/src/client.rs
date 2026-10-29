use std::{
    future::IntoFuture,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use papo_core::{Response, ResponseMatcher, TextKind};
use papo_proto::{Ctcp, Typing, time};
use tokio::sync::{mpsc, oneshot};

use crate::{
    Error, Feature, InvalidInput, Line, Unsupported, UnsupportedReason,
    driver::{Command, CoreRequest},
    event_impl,
    state::State,
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
/// client.quit("example complete").await?;
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

/// Server-assigned metadata for a sent chat message.
///
/// Applications that log every chat message must log their own sends from this value. The
/// correlated server copy is consumed and is not also emitted as a [`crate::Message`] event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Sent {
    /// IRC conversation to which the message was sent.
    ///
    /// See [`crate::Conversation`] for status-prefixed line guidance.
    pub conversation: crate::Conversation,
    /// The lowest-ranked `STATUSMSG` prefix on the wire address.
    ///
    /// See [`crate::Conversation`] for status-prefixed line guidance.
    pub status: Option<crate::Prefix>,
    pub(crate) receipt: crate::message_ref::Receipt,
    /// Whether the server confirmed the message, with its assigned metadata when it did.
    pub outcome: SentOutcome,
    /// Compatibility fallbacks applied while sending.
    pub fallbacks: Vec<Fallback>,
}

/// The server-confirmation outcome of a sent chat message.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SentOutcome {
    /// Every logical message line was echoed with a server-assigned identifier.
    #[non_exhaustive]
    Confirmed {
        /// One identifier and exact sent text for each logical line after papo's splitting.
        ///
        /// Segments in one `draft/multiline` batch form one logical line. For long messages, papo
        /// uses multiline concatenation so length-based splits do not introduce line breaks.
        lines: Vec<(crate::MessageId, String)>,
        /// Server time from the first labelled echo that supplied one.
        server_time: Option<SystemTime>,
    },
    /// The server did not confirm the message with echoes and identifiers.
    ///
    /// This happens when `echo-message` and `labeled-response` are not both available. The send
    /// resolves after its last wire line is written, so it cannot be used for reactions or
    /// redaction.
    Unconfirmed,
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
    _lease: Option<crate::direct::MonitorLease>,
}

impl std::fmt::Debug for SendMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("SendMessage", formatter)
    }
}

/// Builds a channel join request.
#[must_use = "requests do nothing unless awaited"]
pub struct JoinChannel {
    client: Client,
    channel: crate::target::sealed::Parts<'static>,
    key: Option<crate::Secret>,
}

impl std::fmt::Debug for JoinChannel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("JoinChannel", formatter)
    }
}

impl JoinChannel {
    /// Supplies the channel key without exposing it through `Debug` output.
    pub fn key(mut self, key: impl Into<crate::Secret>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl IntoFuture for JoinChannel {
    type Output = Result<(), Error>;
    type IntoFuture = crate::RequestFuture<()>;

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

impl SendMessage {
    /// Marks this message as a reply to `message` when replies are available.
    ///
    /// The referenced identifier does not constrain the target's audience. See
    /// [`crate::Conversation`] for status-prefixed line guidance.
    pub fn reply_to(mut self, message: impl Into<crate::MessageId>) -> Self {
        self.reply_to = Some(message.into());
        self
    }

    /// Supplies channel context for a private message.
    pub fn channel_context(mut self, channel: impl crate::AsChannel) -> Self {
        self.channel_context = Some(crate::AsChannel::__parts(&channel).into_owned().into());
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
    pub fn tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.push((key.into(), value.into()));
        self
    }
}

impl IntoFuture for SendMessage {
    type Output = Result<Sent, Error>;
    type IntoFuture = crate::RequestFuture<Sent>;

    fn into_future(self) -> Self::IntoFuture {
        crate::RequestFuture::new(async move {
            if let Some(error) = self.invalid_input {
                return Err(error.into());
            }
            let target = self.client.resolve_send("target", self.target).await?;
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
                .send_text(
                    self.kind,
                    &target.text,
                    &self.text,
                    tags,
                    fallbacks,
                    self.action,
                    target.guard,
                )
                .await
        })
    }
}

impl Sent {
    /// Returns whether and how the server confirmed the sent message.
    #[must_use]
    pub const fn outcome(&self) -> &SentOutcome {
        &self.outcome
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

    pub(crate) fn unconfirmed(
        conversation: crate::Conversation,
        status: Option<crate::Prefix>,
        receipt: crate::message_ref::Receipt,
        fallbacks: Vec<Fallback>,
    ) -> Self {
        Self {
            conversation,
            status,
            receipt,
            outcome: SentOutcome::Unconfirmed,
            fallbacks,
        }
    }

    pub(crate) fn from_confirmations(
        confirmations: &[papo_core::SentInfo],
        texts: Vec<String>,
        conversation: crate::Conversation,
        status: Option<crate::Prefix>,
        mut receipt: crate::message_ref::Receipt,
        fallbacks: Vec<Fallback>,
    ) -> Self {
        let server_time = confirmations
            .iter()
            .find_map(|sent| sent.server_time.as_deref().and_then(time::parse));
        let msgids = confirmations
            .iter()
            .map(|sent| crate::MessageId::from(sent.msgid.clone()))
            .collect::<Vec<_>>();
        let confirmed = confirmations.len() == texts.len();
        let outcome = if confirmed {
            SentOutcome::Confirmed {
                lines: msgids.iter().cloned().zip(texts).collect(),
                server_time,
            }
        } else {
            SentOutcome::Unconfirmed
        };
        if confirmed {
            receipt.set_reference(msgids, None);
        }
        Self {
            conversation,
            status,
            receipt,
            outcome,
            fallbacks,
        }
    }
}

impl Client {
    async fn resolve_send(
        &self,
        argument: &'static str,
        parts: crate::target::sealed::Parts<'static>,
    ) -> Result<ResolvedSend, Error> {
        let mut recipient = parts
            .recipient
            .as_ref()
            .map(crate::message_ref::RecipientGuard::to_core);
        let channel = parts.channel.as_deref().cloned();
        let mut lease = None;
        let mut text = self.resolve(crate::target::Role::Send(argument), parts)?;
        if let Some(guard) = &recipient {
            match self.state().resolve_recipient(guard) {
                papo_core::RecipientStatus::Current(nick) => text = nick.to_string(),
                papo_core::RecipientStatus::Gone => return Err(Error::TargetGone),
                papo_core::RecipientStatus::Unknown => {
                    let expected = guard.__account().clone();
                    crate::direct_runtime::require_monitor_slot(&self.state(), &text)?;
                    let acquired = crate::direct_runtime::command(
                        self,
                        text.clone(),
                        Some(expected.clone()),
                        true,
                    )
                    .await?;
                    let response = match self
                        .correlated(
                            Line::new("WHO").with_params([text.clone(), "%na".to_owned()]),
                            Some(ResponseMatcher::Who(text.clone())),
                        )
                        .await
                    {
                        Ok(response) => response,
                        Err(error) => {
                            crate::direct_runtime::release(self, acquired).await;
                            return Err(error);
                        }
                    };
                    let actual = match crate::direct_runtime::who_account(self, &text, &response) {
                        Ok(account) => account,
                        Err(error) => {
                            crate::direct_runtime::release(self, acquired).await;
                            return Err(error);
                        }
                    };
                    if !matches!(
                        (&expected, &actual),
                        (crate::Account::LoggedIn(left), crate::Account::LoggedIn(right))
                            if left == right
                    ) {
                        crate::direct_runtime::release(self, acquired).await;
                        return Err(Error::TargetGone);
                    }
                    match crate::direct_runtime::command(self, text.clone(), Some(actual), false)
                        .await
                    {
                        Ok(refreshed) => {
                            lease = Some(crate::direct::lease(self, acquired));
                            recipient = Some(refreshed);
                        }
                        Err(error) => {
                            crate::direct_runtime::release(self, acquired).await;
                            return Err(error);
                        }
                    }
                }
            }
        }
        Ok(ResolvedSend {
            text,
            guard: recipient
                .map(papo_core::TargetGuard::Recipient)
                .or_else(|| channel.map(papo_core::TargetGuard::Channel)),
            _lease: lease,
        })
    }

    pub(crate) fn resolve<'a>(
        &self,
        role: crate::target::Role,
        parts: impl Into<crate::target::sealed::Parts<'a>>,
    ) -> Result<String, Error> {
        let parts = parts.into();
        if parts.origin != crate::Origin::NONE && parts.origin != self.origin {
            return Err(InvalidInput::ForeignReference.into());
        }
        let mut text = parts.text.into_owned();
        let state = self.state();
        let name_rules = state
            .known_name_rules()
            .ok_or(InvalidInput::NameRulesUnknown)?;
        if let Some(channel) = parts.channel.as_deref() {
            let current = state.resolve_channel(channel).ok_or(Error::TargetGone)?;
            if current != *channel.__channel() {
                let prefix = text
                    .strip_suffix(channel.__channel().as_ref())
                    .ok_or(Error::TargetGone)?;
                text = format!("{prefix}{current}");
            }
        }
        if let Some(recipient) = parts.recipient.as_ref() {
            match state.resolve_recipient(&recipient.to_core()) {
                papo_core::RecipientStatus::Current(nick) => {
                    if matches!(role, crate::target::Role::Nick(_)) {
                        text = nick.to_string();
                    }
                }
                papo_core::RecipientStatus::Gone => {
                    return Err(Error::TargetGone);
                }
                papo_core::RecipientStatus::Unknown
                    if matches!(role, crate::target::Role::Nick(_)) =>
                {
                    return Err(Error::TargetGone);
                }
                papo_core::RecipientStatus::Unknown => {}
            }
        }
        let result = match role {
            crate::target::Role::Send(_) => {
                state.parse_target_inner(&text).and_then(|(_, target)| {
                    if matches!(target, crate::Conversation::Channel(_))
                        || parts.recipient.is_some()
                    {
                        Ok(())
                    } else {
                        Err(crate::InvalidIdentifier::InvalidPrefix(
                            text.chars().next().unwrap_or('\0'),
                        ))
                    }
                })
            }
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
    pub fn builder(endpoint: impl crate::IntoEndpoint, nick: impl Into<String>) -> crate::Builder {
        crate::Builder::from_endpoint(endpoint, nick)
    }

    /// Returns the latest immutable state snapshot.
    ///
    /// The returned snapshot can be held across `.await` points.
    #[must_use]
    pub fn state(&self) -> Arc<State> {
        Arc::clone(
            &self
                .state
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Returns the client's current nickname.
    #[must_use]
    pub fn nick(&self) -> crate::Nick {
        self.state().nick().to_owned()
    }

    pub(crate) fn require_feature(state: &State, feature: Feature) -> Result<(), Error> {
        match state.unsupported_reason(feature) {
            Some(reason) => Err(Error::Unsupported(Unsupported::new(feature, reason))),
            None => Ok(()),
        }
    }
    /// Disconnects the current transport and starts a new connection immediately.
    ///
    /// This skips any reconnect backoff. A builder configured with
    /// [`crate::Builder::stream`] cannot create another custom stream.
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
    /// Returns [`Error::Disconnected`] for a custom stream or after the driver stops.
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

    /// Queues one IRC protocol line for transmission without typed target guards.
    ///
    /// The future resolves only after the entire line has been written.
    /// Sending is subject to the client's normal outbound rate limit.
    /// Construct the [`Line`] with [`Line::new`] and [`Line::param`], or explicitly parse text for
    /// a `/quote`-style input. Parsed parameters are ordinary text: an application that puts a
    /// credential in a parsed raw line chooses readable diagnostics and no secret-buffer wiping.
    /// Prefer a typed client method or pass [`crate::Secret`] to [`Line::param`] for credentials.
    /// This is the explicit escape hatch for deliberately unguarded direct-message targets.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] when the line cannot be represented on
    /// the wire, or [`Error::Disconnected`] if the driver stops before writing it.
    pub fn send_raw(&self, line: Line) -> crate::RequestFuture<()> {
        let client = self.clone();
        crate::RequestFuture::new(async move { client.send_message(line).await })
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
    /// verified, or [`Error::Disconnected`] if the driver stops before writing the reply.
    pub fn ctcp_reply(
        &self,
        query: &crate::event::CtcpQuery,
        text: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let target = crate::target::sealed::Parts {
            text: std::borrow::Cow::Owned(query.actor.nick().to_string()),
            origin: crate::message_ref::conversation_origin(&query.conversation),
            recipient: Some(crate::message_ref::actor_recipient(query)),
            channel: None,
        };
        let text = text.into();
        let command = query.command.clone();
        let body = Ctcp::build(&command, (!text.is_empty()).then_some(text.as_str()));
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let body = body.ok_or(InvalidInput::InvalidCtcp)?;
            let target = client
                .resolve_send("ctcp_reply: query", target.into_owned())
                .await?;
            let reply = Line::notice(&target.text, body);
            client.send_message_guarded(reply, target.guard).await
        })
    }

    /// Sends one IRC protocol line and waits for its labelled response.
    ///
    /// Response lines are returned here rather than emitted by the event
    /// stream. Their state changes are still applied before this future
    /// completes.
    ///
    /// Construct the [`Line`] with [`Line::new`] and [`Line::param`], or explicitly parse text for
    /// a `/quote`-style input. Parsed parameters are ordinary text: an application that puts a
    /// credential in a parsed raw line chooses readable diagnostics and no secret-buffer wiping.
    /// Prefer a typed client method or pass [`crate::Secret`] to [`Line::param`] for credentials.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when `labeled-response` is unavailable;
    /// raw requests have no unlabelled fallback. Also returns an error when the
    /// line cannot be represented on the wire, the request limit is reached,
    /// the response times out, or the driver stops.
    pub fn request_raw(&self, line: Line) -> crate::RequestFuture<Vec<Line>> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let response = client.correlated(line, None).await?;
            Ok(response.lines)
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

    pub(crate) async fn correlated(
        &self,
        message: Line,
        matcher: Option<ResponseMatcher>,
    ) -> Result<Response, Error> {
        self.correlated_guarded(message, matcher, None).await
    }

    async fn correlated_guarded(
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

    /// Sends a message to a channel or guarded [`crate::DirectRef`].
    ///
    /// With `echo-message` and `labeled-response`, the future resolves with the server's copy,
    /// which is not also emitted as an event. Applications that log every message must log their
    /// own sends from the returned [`Sent`]. Without both capabilities, it resolves after the last
    /// line is written and has no server-assigned metadata.
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
    /// A typed nickname is not a message target; capture a [`crate::DirectRef`] first:
    ///
    /// ```compile_fail
    /// # fn unguarded(client: &papo::Client, nick: &papo::Nick) {
    /// let _ = client.privmsg(nick, "unguarded");
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for empty text, a nickname passed as text, or an
    /// unencodable message, and [`Error::TargetGone`] when a guarded recipient has left or its
    /// nickname has changed hands.
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

    /// Sends a notice to a channel, guarded direct recipient, or return address.
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

    /// Sends a CTCP ACTION to a channel, guarded direct recipient, or return address.
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

    /// Sends a private message threaded as a reply to `message` when supported.
    ///
    /// Passing the received line or [`Sent`] preserves its return address. See
    /// [`crate::Conversation`] for status-prefixed line guidance.
    /// A notice is not a reply input:
    ///
    /// ```compile_fail
    /// # fn notice(client: &papo::Client, notice: &papo::event::Notice) {
    /// let _ = client.reply(notice, "automatic response");
    /// # }
    /// ```
    ///
    /// A private return address follows a tracked sender's nickname changes. If the sender is not
    /// tracked, papo verifies its captured account with `WHO`; a mismatch or identity that cannot
    /// be verified is refused rather than sent to a bare nickname.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TargetGone`] in the private-recipient case described above. It returns an
    /// error if the text is empty or invalid, or the driver has stopped.
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
    /// Passing the received line or [`Sent`] preserves its return address; see
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
    pub fn react(
        &self,
        message: impl Into<crate::MessageRef>,
        emoji: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        self.reaction(message.into(), emoji.into(), "+draft/react")
    }

    /// Removes `emoji` as a reaction to `message`.
    ///
    /// `emoji` must exactly match the added reaction; papo performs no Unicode normalization.
    /// Passing the received line or [`Sent`] preserves its return address; see
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
    pub fn unreact(
        &self,
        message: impl Into<crate::MessageRef>,
        emoji: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        self.reaction(message.into(), emoji.into(), "+draft/unreact")
    }

    fn reaction(
        &self,
        message: crate::MessageRef,
        emoji: String,
        reaction_tag: &'static str,
    ) -> crate::RequestFuture<()> {
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
            let target = client
                .resolve_send(
                    "reaction: message",
                    crate::Target::__parts(&message).into_owned(),
                )
                .await?;
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
            let line = Line::tagmsg(target.text)
                .with_tag(reply_tag, message_id.as_ref())
                .with_tag(reaction_tag, emoji);
            if state.supports(Feature::LabeledResponse) {
                let response = client.correlated_guarded(line, None, target.guard).await?;
                crate::requests::empty(&response)
            } else {
                client.send_message_guarded(line, target.guard).await
            }
        })
    }

    /// Sends a typing state to a channel, guarded direct recipient, or return address.
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
    pub fn typing(&self, target: impl crate::Target, state: Typing) -> crate::RequestFuture<()> {
        let client = self.clone();
        let target = crate::Target::__parts(&target).into_owned();
        crate::RequestFuture::new(async move {
            let target = client.resolve_send("typing: target", target).await?;
            let snapshot = client.state();
            Self::require_feature(&snapshot, Feature::Typing)?;
            let (result, receiver) = oneshot::channel();
            client
                .commands
                .send(Command::Typing {
                    target: target.text,
                    state,
                    guard: target.guard,
                    result,
                })
                .await
                .map_err(|_| Error::connection_lost())?;
            receiver.await.map_err(|_| Error::connection_lost())?
        })
    }

    /// Redacts a message from a conversation's history.
    ///
    /// Passing the received line or [`Sent`] preserves its return address. See
    /// [`crate::Conversation`] for status-prefixed line guidance.
    /// A sent message split across multiple lines is redacted in send order. Redaction stops at
    /// the first error, and any earlier lines remain redacted.
    /// An empty `reason` means no reason.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`Feature::Redaction`] with reason
    /// [`UnsupportedReason::NotOffered`] or [`UnsupportedReason::DisabledByClient`]. A refusal
    /// returns [`Error::Server`], currently classified as [`crate::Refusal::Other`]. It can also
    /// return [`Error::InvalidInput`], [`Error::TooManyPendingRequests`], [`Error::Timeout`], or
    /// [`Error::Disconnected`]. A reference without a server message identifier returns
    /// [`InvalidInput::UnusableMessageReference`].
    /// A private reference whose nickname is proven to belong to another user returns
    /// [`Error::TargetGone`].
    pub fn redact(
        &self,
        message: impl Into<crate::MessageRef>,
        reason: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let message = message.into();
        let reason = reason.into();
        crate::RequestFuture::new(async move {
            if message.msgids().is_empty() {
                return Err(InvalidInput::UnusableMessageReference.into());
            }
            let target = client
                .resolve_send(
                    "redact: message",
                    crate::Target::__parts(&message).into_owned(),
                )
                .await?;
            let state = client.state();
            Self::require_feature(&state, Feature::Redaction)?;
            drop(state);
            let commands = client.commands.clone();
            let (result, receiver) = oneshot::channel();
            let task = Box::pin(async move {
                let outcome = async {
                    for message_id in message.msgids() {
                        message_id.check().map_err(|error| {
                            InvalidInput::invalid_name(message_id.to_string(), error)
                        })?;
                        let mut params = vec![target.text.clone(), message_id.to_string()];
                        if !reason.is_empty() {
                            params.push(reason.clone());
                        }
                        let response = client
                            .correlated_guarded(
                                Line::new("REDACT").with_params(params),
                                None,
                                target.guard.clone(),
                            )
                            .await?;
                        crate::requests::empty(&response)?;
                    }
                    Ok(())
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

    #[expect(clippy::too_many_arguments)]
    pub(crate) async fn send_text(
        &self,
        kind: TextKind,
        target: &str,
        text: &str,
        tags: Vec<(String, String)>,
        fallbacks: Vec<Fallback>,
        action: bool,
        guard: Option<papo_core::TargetGuard>,
    ) -> Result<Sent, Error> {
        let state = self.state();
        let (status, conversation) = state
            .parse_target_inner(target)
            .map_err(|reason| InvalidInput::invalid_name(target, reason))?;
        let receipt = state.sent_receipt(&conversation, target.to_owned(), guard.as_ref());
        if !state.is_connected() {
            let text = if action {
                Ctcp::action(text).ok_or(InvalidInput::InvalidCtcp)?
            } else {
                text.to_owned()
            };
            let mut message = match kind {
                TextKind::Privmsg => Line::privmsg(target, text),
                TextKind::Notice => Line::notice(target, text),
            };
            for (key, value) in tags {
                message = message.with_tag(key, value);
            }
            self.send_message_guarded(message, guard).await?;
            return Ok(Sent::unconfirmed(conversation, status, receipt, fallbacks));
        }
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::SendText {
                kind,
                target: target.to_owned(),
                conversation,
                status,
                receipt,
                text: text.to_owned(),
                tags,
                fallbacks,
                action,
                guard,
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
    /// Returns [`Error::Server`] with [`crate::Refusal::NoSuchNick`],
    /// [`crate::Refusal::NoSuchServer`], [`crate::Refusal::TryAgain`], or
    /// [`crate::Refusal::Other`] when refused. It can also return [`Error::InvalidInput`],
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    /// ```compile_fail
    /// # fn swapped(client: &papo::Client, channel: &papo::ChannelName) {
    /// let _ = client.whois(channel);
    /// # }
    /// ```
    pub fn whois(&self, nick: impl crate::AsNick) -> crate::RequestFuture<crate::Whois> {
        let client = self.clone();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        crate::RequestFuture::new(async move {
            let nick = client.resolve(crate::target::Role::Nick("whois: nick"), nick)?;
            let nick = client
                .state()
                .known_name_rules()
                .ok_or(InvalidInput::NameRulesUnknown)?
                .nick(nick);
            let response = client
                .correlated(
                    Line::new("WHOIS").with_params([nick.to_string()]),
                    Some(ResponseMatcher::Whois(nick.to_string())),
                )
                .await?;
            crate::requests::whois(
                response,
                &nick,
                client.state().isupport().inner(),
                client.origin(),
            )
        })
    }

    /// Requests the users matching a channel or mask.
    ///
    /// When the argument names a joined channel, the reply refreshes that channel's member facts
    /// in [`crate::State`] and emits [`crate::Event::MembersRefreshed`] at numeric `315`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::TryAgain`] or
    /// [`crate::Refusal::Other`] when refused. It can also return [`Error::InvalidInput`],
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
            crate::requests::who(
                response,
                state.isupport().inner(),
                client.origin(),
                extended,
            )
        })
    }

    /// Joins a channel and waits for its topic and member list to synchronize.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::TooManyChannels`], [`crate::Refusal::TemporarilyUnavailable`],
    /// [`crate::Refusal::ChannelIsFull`], [`crate::Refusal::InviteOnlyChannel`],
    /// [`crate::Refusal::BannedFromChannel`], [`crate::Refusal::BadChannelKey`],
    /// [`crate::Refusal::BadChannelMask`], [`crate::Refusal::SecureOnlyChannel`],
    /// [`crate::Refusal::TryAgain`], or [`crate::Refusal::Other`] when refused. It can also
    /// return [`Error::InvalidInput`], [`Error::TooManyPendingRequests`], [`Error::Timeout`], or
    /// [`Error::Disconnected`].
    /// ```compile_fail
    /// # fn swapped(client: &papo::Client, nick: &papo::Nick) {
    /// let _ = client.join(nick);
    /// # }
    /// ```
    pub fn join(&self, channel: impl crate::AsChannel) -> JoinChannel {
        JoinChannel {
            client: self.clone(),
            channel: crate::AsChannel::__parts(&channel).into_owned().into(),
            key: None,
        }
    }

    async fn join_channel(
        &self,
        channel: crate::ChannelName,
        key: Option<crate::Secret>,
    ) -> Result<(), Error> {
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
        if self.state().channel(&channel).is_none() {
            return Err(Error::unexpected_reply("JOIN"));
        }
        if !self
            .state()
            .channel(&channel)
            .is_some_and(papo_core::Channel::is_synced)
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
        Ok(())
    }

    /// Leaves a channel.
    ///
    /// An empty `reason` means no reason.
    ///
    /// While disconnected, this removes the channel from the automatic rejoin set and returns
    /// [`Error::Disconnected`] without writing a `PART`. If a bouncer restores the channel after
    /// reconnection, papo sends `PART` then.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::NotOnChannel`], [`crate::Refusal::TryAgain`], or
    /// [`crate::Refusal::Other`] when refused. It can also return [`Error::InvalidInput`],
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub fn part(
        &self,
        channel: impl crate::AsChannel,
        reason: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let channel = crate::AsChannel::__parts(&channel).into_owned();
        let reason = reason.into();
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
            crate::requests::empty(&response)
        })
    }

    /// Removes `nick` from `channel`.
    ///
    /// A [`crate::DirectRef`] follows its captured identity; text or [`crate::Nick`] addresses
    /// whoever currently holds that nickname. An empty `reason` means no reason.
    ///
    /// ```compile_fail
    /// # fn swapped(
    /// #     client: &papo::Client,
    /// #     nick: &papo::Nick,
    /// #     channel: &papo::ChannelName,
    /// # ) {
    /// let _ = client.kick(nick, channel, "spam");
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a typed server refusal, invalid-input error, timeout, or disconnect.
    pub fn kick(
        &self,
        channel: impl crate::AsChannel,
        nick: impl crate::AsNick,
        reason: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let channel = crate::AsChannel::__parts(&channel).into_owned();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        let reason = reason.into();
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
    /// A [`crate::DirectRef`] is checked again after ban confirmation. Text or [`crate::Nick`]
    /// deliberately addresses whoever currently holds that nickname.
    /// The ban remains set when the kick is refused. Leaving or changing nickname during ban
    /// confirmation produces [`Error::TargetGone`] in [`crate::KickbanOutcome::kick`].
    /// An empty `reason` means no reason.
    ///
    /// ```compile_fail
    /// # fn mask_is_not_a_reason(client: &papo::Client, reason: &papo::Mask) {
    /// let _ = client.kickban("#papo", "mallory", "*!*@gateway.example", reason);
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unconfirmed`] without sending `KICK` when the ban cannot be proved. A ban
    /// refusal and the errors documented by [`Client::kick`] are returned before an outcome.
    pub fn kickban(
        &self,
        channel: impl crate::AsChannel,
        nick: impl crate::AsNick,
        ban: impl Into<crate::BanTarget>,
        reason: impl Into<String>,
    ) -> crate::RequestFuture<crate::KickbanOutcome> {
        let client = self.clone();
        let channel = crate::AsChannel::__parts(&channel).into_owned();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        let ban = ban.into();
        let reason = reason.into();
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
                .channel(&channel)
                .is_some_and(|joined| joined.member(&resolved_nick).is_some());
            drop(state);
            let commands = client.commands.clone();
            let (result, receiver) = oneshot::channel();
            let task = Box::pin(async move {
                let outcome = async {
                    if !already_set {
                        let response = client
                            .correlated(
                                Line::new("MODE").with_params([
                                    channel.as_ref(),
                                    "+b",
                                    ban.as_ref(),
                                ]),
                                Some(ResponseMatcher::Mode(channel.to_string())),
                            )
                            .await?;
                        let already_set_reply = response.error().is_some_and(|error| {
                            error.refusal() == crate::Refusal::ListModeAlreadySet
                        });
                        if let Some(error) = response.error()
                            && !already_set_reply
                        {
                            return Err(Error::server(error));
                        }
                        let state = client.state();
                        let confirmed = already_set_reply
                            || crate::mode::confirmed_ban(&response.lines, &state, &channel, &ban)
                            || crate::mode::tracked_ban(&state, &channel, &ban);
                        let needs_fetch = !confirmed
                            && state
                                .channel(&channel)
                                .is_none_or(|joined| joined.list(crate::ListMode::Ban).is_none());
                        drop(state);
                        if !confirmed {
                            if !needs_fetch {
                                return Err(Error::Unconfirmed);
                            }
                            let entries = client.list_modes(&channel, crate::ListMode::Ban).await?;
                            let state = client.state();
                            if !entries
                                .iter()
                                .any(|entry| state.same_list_entry(&entry.mask, &ban))
                            {
                                return Err(Error::Unconfirmed);
                            }
                        }
                        if target_was_present
                            && client
                                .state()
                                .channel(&channel)
                                .is_none_or(|joined| joined.member(&resolved_nick).is_none())
                        {
                            return Ok(crate::KickbanOutcome {
                                kick: Err(Error::TargetGone),
                            });
                        }
                    }

                    Ok(crate::KickbanOutcome {
                        kick: kick_resolved(&client, channel, nick, reason).await,
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

    /// Invites a guarded user identity to `channel`.
    ///
    /// Capture a typed nickname with [`Client::direct`], or use [`Client::direct_to`] to retain a
    /// received message actor's identity.
    ///
    /// ```compile_fail
    /// # fn unguarded(client: &papo::Client, nick: &papo::Nick, channel: &papo::ChannelName) {
    /// let _ = client.invite(nick, channel);
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a typed server refusal, invalid-input error, timeout, or disconnect.
    pub fn invite(
        &self,
        nick: &crate::DirectRef,
        channel: impl crate::AsChannel,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let channel = crate::AsChannel::__parts(&channel).into_owned();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        crate::RequestFuture::new(async move {
            let nick = client.resolve(crate::target::Role::Nick("invite: nick"), nick)?;
            let channel =
                client.resolve(crate::target::Role::Channel("invite: channel"), channel)?;
            let state = client.state();
            let names = state
                .known_name_rules()
                .ok_or(InvalidInput::NameRulesUnknown)?;
            let channel = names.channel(channel);
            let nick = names.nick(nick);
            let response = client
                .correlated(
                    Line::new("INVITE").with_params([nick.to_string(), channel.to_string()]),
                    Some(ResponseMatcher::__invite(
                        channel.to_string(),
                        nick.to_string(),
                    )),
                )
                .await?;
            crate::requests::empty(&response)
        })
    }

    /// Changes this client's nickname.
    ///
    /// An empty nickname is invalid input.
    ///
    /// # Errors
    ///
    /// Returns a typed server refusal, invalid-input error, timeout, or disconnect.
    /// ```compile_fail
    /// # fn swapped(client: &papo::Client, channel: &papo::ChannelName) {
    /// let _ = client.set_nick(channel);
    /// # }
    /// ```
    pub fn set_nick(&self, nick: impl crate::AsNick) -> crate::RequestFuture<()> {
        let client = self.clone();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        crate::RequestFuture::new(async move {
            let nick = client.resolve(crate::target::Role::Nick("set_nick: nick"), nick)?;
            let nick = client
                .state()
                .known_name_rules()
                .ok_or(InvalidInput::NameRulesUnknown)?
                .nick(nick);
            let response = client
                .correlated(
                    Line::new("NICK").with_params([nick.to_string()]),
                    Some(ResponseMatcher::Nick(nick.to_string())),
                )
                .await?;
            crate::requests::empty(&response)
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

    /// Changes this client's realname through the `setname` capability.
    ///
    /// An empty realname is invalid input.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when `setname` is unavailable, or a typed server refusal,
    /// invalid-input error, timeout, or disconnect.
    pub fn set_realname(&self, realname: impl Into<String>) -> crate::RequestFuture<()> {
        let client = self.clone();
        let realname = realname.into();
        crate::RequestFuture::new(async move {
            if realname.is_empty() {
                return Err(InvalidInput::EmptyText.into());
            }
            Self::require_feature(&client.state(), Feature::SetName)?;
            let response = client
                .correlated(
                    Line::new("SETNAME").with_params([realname.clone()]),
                    Some(ResponseMatcher::SetName(realname)),
                )
                .await?;
            crate::requests::empty(&response)
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
        password: impl Into<crate::Secret>,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let name = name.into();
        let password = password.into();
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

    /// Sets the topic for `channel`.
    ///
    /// An empty `topic` is invalid input. Use [`Client::clear_topic`] to clear it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::NotOnChannel`], [`crate::Refusal::ChannelOperatorPrivilegesNeeded`],
    /// [`crate::Refusal::TryAgain`], or [`crate::Refusal::Other`] when refused. It can also
    /// return [`Error::InvalidInput`], [`Error::TooManyPendingRequests`], [`Error::Timeout`], or
    /// [`Error::Disconnected`].
    pub fn set_topic(
        &self,
        channel: impl crate::AsChannel,
        topic: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        self.change_topic(
            crate::AsChannel::__parts(&channel).into_owned().into(),
            topic.into(),
            false,
        )
    }

    /// Clears the topic for `channel`.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Client::set_topic`].
    pub fn clear_topic(&self, channel: impl crate::AsChannel) -> crate::RequestFuture<()> {
        self.change_topic(
            crate::AsChannel::__parts(&channel).into_owned().into(),
            String::new(),
            true,
        )
    }

    fn change_topic(
        &self,
        channel: crate::target::sealed::Parts<'static>,
        topic: String,
        clear: bool,
    ) -> crate::RequestFuture<()> {
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
            crate::requests::empty(&response)
        })
    }

    /// Starts a request for visible channels.
    ///
    /// At most 10,000 entries are retained.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::TryAgain`] or
    /// [`crate::Refusal::Other`] when refused. It can also return [`Error::InvalidInput`],
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub fn list(&self) -> crate::ListRequest {
        crate::ListRequest::new(self.clone())
    }

    /// Requests a clean disconnect and waits for the peer to close.
    ///
    /// The wait is limited to ten seconds. The driver still completes cleanly
    /// when that limit is reached. An I/O failure while waiting for closure
    /// also resolves to `Ok(())`. A timed-out acknowledgement is recorded in
    /// the final [`crate::Event::Disconnected`].
    /// An empty `reason` means no reason.
    ///
    /// # Errors
    ///
    /// Returns an error only if the QUIT message is invalid.
    pub fn quit(&self, reason: impl Into<String>) -> crate::RequestFuture<()> {
        let client = self.clone();
        let reason = reason.into();
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

async fn kick_resolved(
    client: &Client,
    channel: crate::ChannelName,
    nick: crate::target::sealed::Parts<'static>,
    reason: String,
) -> Result<(), Error> {
    let nick = client.resolve(crate::target::Role::Nick("kick: nick"), nick)?;
    let state = client.state();
    let nick = state
        .known_name_rules()
        .ok_or(InvalidInput::NameRulesUnknown)?
        .nick(nick);
    validate_kick_reason(&state, &reason)?;
    let mut params = vec![channel.to_string(), nick.to_string()];
    if !reason.is_empty() {
        params.push(reason);
    }
    let response = client
        .correlated(
            Line::new("KICK").with_params(params),
            Some(ResponseMatcher::__kick(
                channel.to_string(),
                nick.to_string(),
            )),
        )
        .await?;
    crate::requests::empty(&response)
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
