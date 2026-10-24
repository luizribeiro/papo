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
/// let (client, _events) = papo::Client::builder("ircs://irc.example.net")
///     .nick("papo-client")
///     .connect()
///     .await?;
/// client.privmsg("#papo", "hello from the docs").await?;
/// # Ok(())
/// # }
/// ```
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
    /// `STATUSMSG` prefixes on the wire address, in wire order.
    ///
    /// See [`crate::Conversation`] for status-prefixed line guidance.
    pub status: Option<Vec<char>>,
    /// Server-assigned identifier from a labelled echo.
    pub msgid: Option<crate::MessageId>,
    /// Server-assigned identifiers for every confirmed IRC message line, in send order.
    pub line_msgids: Vec<crate::MessageId>,
    /// Server time from a labelled echo.
    pub server_time: Option<SystemTime>,
    /// Compatibility fallbacks applied while sending.
    pub fallbacks: Vec<Fallback>,
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
    target: String,
    text: String,
    reply_to: Option<crate::MessageId>,
    channel_context: Option<crate::ChannelName>,
    tags: Vec<(String, String)>,
    action: bool,
    invalid_input: Option<InvalidInput>,
    recipient: Option<crate::message_ref::RecipientGuard>,
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
    channel: crate::ChannelName,
    key: Option<crate::Secret>,
    invalid_input: Option<InvalidInput>,
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
            if let Some(error) = self.invalid_input {
                return Err(error.into());
            }
            self.client.join_channel(self.channel, self.key).await
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
    pub fn channel_context(mut self, channel: impl crate::Target) -> Self {
        let (channel, error) = self.client.target_input(channel);
        self.invalid_input = self.invalid_input.or(error);
        self.channel_context = Some(self.client.state().channel_name(channel));
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
            let state = self.client.state();
            if self
                .recipient
                .as_ref()
                .is_some_and(|recipient| state.recipient_changed(recipient))
            {
                return Err(Error::TargetGone);
            }
            for (key, _) in &self.tags {
                if !key.starts_with('+') {
                    return Err(InvalidInput::InvalidMessage(format!(
                        "client-only message tag {key:?} must start with '+'"
                    ))
                    .into());
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
            state
                .parse_target_inner(&self.target)
                .map_err(|reason| InvalidInput::invalid_name(self.target.clone(), reason))?;
            if let Some(message) = &self.reply_to {
                message
                    .check()
                    .map_err(|reason| InvalidInput::invalid_name(message.to_string(), reason))?;
            }
            if let Some(channel) = &self.channel_context {
                channel
                    .check(state.isupport().inner())
                    .map_err(|reason| InvalidInput::invalid_name(channel.to_string(), reason))?;
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
                if let Some(channel) = self.channel_context.as_ref()
                    && let Some(tag) = event_impl::allowed_alias(
                        &denied,
                        "+channel-context",
                        "+draft/channel-context",
                    )
                {
                    tags.push((tag.to_owned(), channel.to_string()));
                } else if self.channel_context.is_some() {
                    fallbacks.push(Fallback::ContextTagDropped);
                }
            } else {
                fallbacks.extend(self.reply_to.map(|_| Fallback::ReplyTagDropped));
                fallbacks.extend(self.channel_context.map(|_| Fallback::ContextTagDropped));
            }
            self.client
                .send_text(
                    self.kind,
                    &self.target,
                    &self.text,
                    tags,
                    fallbacks,
                    self.action,
                )
                .await
        })
    }
}

impl Sent {
    /// Returns the client identity associated with this message.
    #[must_use]
    pub fn origin(&self) -> crate::Origin {
        crate::message_ref::conversation_origin(&self.conversation)
    }

    /// Returns the address that reaches the recipients of the sent message.
    ///
    /// See [`crate::Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn return_address(&self) -> String {
        crate::message_ref::return_address(&self.conversation, self.status.as_deref())
    }

    pub(crate) fn without_echo(
        conversation: crate::Conversation,
        status: Option<Vec<char>>,
        fallbacks: Vec<Fallback>,
    ) -> Self {
        Self {
            conversation,
            status,
            msgid: None,
            line_msgids: Vec::new(),
            server_time: None,
            fallbacks,
        }
    }

    pub(crate) fn from_confirmations(
        confirmations: &[papo_core::SentInfo],
        conversation: crate::Conversation,
        status: Option<Vec<char>>,
        fallbacks: Vec<Fallback>,
    ) -> Self {
        let line_msgids = confirmations
            .iter()
            .map(|sent| crate::MessageId::from(sent.msgid.clone()))
            .collect::<Vec<_>>();
        Self {
            conversation,
            status,
            msgid: line_msgids.first().cloned(),
            line_msgids,
            server_time: confirmations
                .iter()
                .find_map(|sent| sent.server_time.as_deref().and_then(time::parse)),
            fallbacks,
        }
    }
}

impl Client {
    pub(crate) fn target_input(
        &self,
        target: impl crate::Target,
    ) -> (String, Option<InvalidInput>) {
        let (target, origin) = target.__target();
        let error = (origin != crate::Origin::NONE && origin != self.origin)
            .then_some(InvalidInput::ForeignReference);
        (target.to_owned(), error)
    }

    pub(crate) fn check_origin(&self, origin: Option<crate::Origin>) -> Result<(), Error> {
        if origin.is_some_and(|origin| origin != crate::Origin::NONE && origin != self.origin) {
            Err(InvalidInput::ForeignReference.into())
        } else {
            Ok(())
        }
    }

    /// Returns the opaque identity carried by values produced by this client.
    #[must_use]
    pub const fn origin(&self) -> crate::Origin {
        self.origin
    }

    /// Starts configuring a client for `endpoint`.
    ///
    /// Strings accept hostnames and `irc://`, `ircs://`, `ws://`, and `wss://` URLs.
    /// A bare hostname uses TLS on port 6697.
    ///
    /// ```
    /// let builder = papo::Client::builder("ircs://irc.example.net");
    /// # _ = builder;
    /// ```
    pub fn builder(endpoint: impl crate::IntoEndpoint) -> crate::Builder {
        crate::Builder::from_endpoint(endpoint)
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

    /// Queues one IRC protocol line for transmission.
    ///
    /// The future resolves only after the entire line has been written.
    /// Sending is subject to the client's normal outbound rate limit.
    /// Construct the [`Line`] with [`Line::new`] and [`Line::param`], or explicitly parse text for
    /// a `/quote`-style input. Parsed parameters are ordinary text: an application that puts a
    /// credential in a parsed raw line chooses readable diagnostics and no secret-buffer wiping.
    /// Prefer a typed client method or pass [`crate::Secret`] to [`Line::param`] for credentials.
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
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] when the actor is a server or the command or text contains a
    /// CTCP or IRC line delimiter. It returns
    /// [`Error::Disconnected`] if the driver stops before writing the reply.
    pub fn ctcp_reply(
        &self,
        query: &crate::event::CtcpQuery,
        text: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let origin = crate::message_ref::conversation_origin(&query.conversation);
        let text = text.into();
        let reply = papo_core::__ctcp_reply(&query.actor, &query.command, &text);
        let client = self.clone();
        crate::RequestFuture::new(async move {
            client.check_origin(Some(origin))?;
            let reply = reply.ok_or_else(|| {
                InvalidInput::InvalidMessage("invalid CTCP reply query or text".into())
            })?;
            client.send_message(reply).await
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
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::Send { message, result })
            .await
            .map_err(|_| Error::connection_lost())?;
        receiver.await.map_err(|_| Error::connection_lost())?
    }

    pub(crate) async fn correlated(
        &self,
        message: Line,
        matcher: Option<ResponseMatcher>,
    ) -> Result<Response, Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::Request {
                message,
                matcher,
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

    /// Sends a private message to a channel or directly to a user.
    ///
    /// With `echo-message` and `labeled-response`, the future resolves with the server's copy,
    /// which is not also emitted as an event. Applications that log every message must log their
    /// own sends from the returned [`Sent`]. Without both capabilities, it resolves after the last
    /// line is written and has no server-assigned metadata.
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
    pub fn privmsg(&self, target: impl crate::Target, text: impl Into<String>) -> SendMessage {
        let (target, invalid_input) = self.target_input(target);
        SendMessage {
            client: self.clone(),
            kind: TextKind::Privmsg,
            target,
            text: text.into(),
            reply_to: None,
            channel_context: None,
            tags: Vec::new(),
            action: false,
            invalid_input,
            recipient: None,
        }
    }

    /// Sends a notice to a user or channel.
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
        let (target, invalid_input) = self.target_input(target);
        SendMessage {
            client: self.clone(),
            kind: TextKind::Notice,
            target,
            text: text.into(),
            reply_to: None,
            channel_context: None,
            tags: Vec::new(),
            action: false,
            invalid_input,
            recipient: None,
        }
    }

    /// Sends a CTCP ACTION to a user or channel.
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
            request.invalid_input = Some(InvalidInput::InvalidMessage(
                "invalid CTCP ACTION text".to_owned(),
            ));
            request
        }
    }

    /// Sends a private message threaded as a reply to `message` when supported.
    ///
    /// Passing the received line or [`Sent`] preserves its return address. See
    /// [`crate::Conversation`] for status-prefixed line guidance.
    ///
    /// For a private received line, papo returns [`Error::TargetGone`] when current state proves
    /// that another user holds the sender's nickname. It does not follow the sender to a new
    /// nickname. When state cannot identify the current holder, including a sender who shares no
    /// channel and has no account information, the reply is sent.
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
        let (target, msgid, origin, channel_context, recipient) = message.into().into_parts();
        let mut request = self.privmsg(target, text);
        if let Err(Error::InvalidInput(error)) = self.check_origin(origin) {
            request.invalid_input = Some(error);
        }
        request.recipient = recipient;
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
            let (target, message, origin, _, recipient) = message.into_parts();
            client.check_origin(origin)?;
            let Some(message) = message else {
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
            let state = client.state();
            if recipient
                .as_ref()
                .is_some_and(|recipient| state.recipient_changed(recipient))
            {
                return Err(Error::TargetGone);
            }
            Self::require_feature(&state, Feature::Reactions)?;
            state
                .parse_target_inner(&target)
                .map(drop)
                .and_then(|()| message.check())
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
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
            let message = Line::tagmsg(target)
                .with_tag(reply_tag, message.as_ref())
                .with_tag(reaction_tag, emoji);
            if state.supports(Feature::LabeledResponse) {
                let response = client.correlated(message, None).await?;
                crate::requests::empty(&response)
            } else {
                client.send_message(message).await
            }
        })
    }

    /// Sends a typing state to an address.
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
        let (target, invalid_input) = self.target_input(target);
        crate::RequestFuture::new(async move {
            if let Some(error) = invalid_input {
                return Err(error.into());
            }
            let snapshot = client.state();
            Self::require_feature(&snapshot, Feature::Typing)?;
            snapshot
                .parse_target_inner(&target)
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            let (result, receiver) = oneshot::channel();
            client
                .commands
                .send(Command::Typing {
                    target,
                    state,
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
            let (target, message, origin, _, recipient) = message.into_parts();
            client.check_origin(origin)?;
            let Some(message) = message else {
                return Err(InvalidInput::UnusableMessageReference.into());
            };
            let state = client.state();
            if recipient
                .as_ref()
                .is_some_and(|recipient| state.recipient_changed(recipient))
            {
                return Err(Error::TargetGone);
            }
            Self::require_feature(&state, Feature::Redaction)?;
            state
                .parse_target_inner(&target)
                .map(drop)
                .and_then(|()| message.check())
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            let mut params = vec![target.clone(), message.to_string()];
            if !reason.is_empty() {
                params.push(reason);
            }
            let response = client
                .correlated(Line::new("REDACT").with_params(params), None)
                .await?;
            crate::requests::empty(&response)
        })
    }

    pub(crate) async fn send_text(
        &self,
        kind: TextKind,
        target: &str,
        text: &str,
        tags: Vec<(String, String)>,
        fallbacks: Vec<Fallback>,
        action: bool,
    ) -> Result<Sent, Error> {
        let (status, conversation) = self
            .state()
            .parse_target_inner(target)
            .map_err(|reason| InvalidInput::invalid_name(target, reason))?;
        if !self.state().is_connected() {
            let text = if action {
                Ctcp::action(text).ok_or_else(|| {
                    InvalidInput::InvalidMessage("invalid CTCP ACTION text".to_owned())
                })?
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
            self.send_message(message).await?;
            return Ok(Sent::without_echo(conversation, status, fallbacks));
        }
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::SendText {
                kind,
                target: target.to_owned(),
                conversation,
                status,
                text: text.to_owned(),
                tags,
                fallbacks,
                action,
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
    pub fn whois(&self, nick: impl crate::Target) -> crate::RequestFuture<crate::Whois> {
        let client = self.clone();
        let (nick, invalid_input) = self.target_input(nick);
        let nick = self.state().nickname(nick);
        crate::RequestFuture::new(async move {
            if let Some(error) = invalid_input {
                return Err(error.into());
            }
            nick.check(client.state().isupport().inner())
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            let response = client
                .correlated(
                    Line::new("WHOIS").with_params([nick.to_string()]),
                    Some(ResponseMatcher::Whois(nick.to_string())),
                )
                .await?;
            crate::requests::whois(response, &nick, client.state().isupport().inner())
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
    pub fn who(&self, mask: impl crate::Target) -> crate::RequestFuture<Vec<crate::WhoEntry>> {
        let client = self.clone();
        let (mask, invalid_input) = self.target_input(mask);
        let mask = self.state().mask(mask);
        crate::RequestFuture::new(async move {
            if let Some(error) = invalid_input {
                return Err(error.into());
            }
            mask.check()
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
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
            crate::requests::who(response, state.isupport().inner(), extended)
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
    pub fn join(&self, channel: impl crate::Target) -> JoinChannel {
        let (channel, invalid_input) = self.target_input(channel);
        JoinChannel {
            client: self.clone(),
            channel: self.state().channel_name(channel),
            key: None,
            invalid_input,
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

    /// Leaves a channel, omitting an empty `reason` from the wire.
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
        channel: impl crate::Target,
        reason: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let (channel, invalid_input) = self.target_input(channel);
        let channel = self.state().channel_name(channel);
        let reason = reason.into();
        crate::RequestFuture::new(async move {
            if let Some(error) = invalid_input {
                return Err(error.into());
            }
            channel
                .check(client.state().isupport().inner())
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
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

    /// Removes `nick` from `channel`, omitting an empty `reason` from the wire.
    ///
    /// # Errors
    ///
    /// Returns a typed server refusal, invalid-input error, timeout, or disconnect.
    pub fn kick(
        &self,
        channel: impl crate::Target,
        nick: impl crate::Target,
        reason: impl AsRef<str>,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let (channel, channel_error) = self.target_input(channel);
        let (nick, nick_error) = self.target_input(nick);
        let state = self.state();
        let channel = state.channel_name(channel);
        let nick = state.nickname(nick);
        let reason = reason.as_ref().to_owned();
        crate::RequestFuture::new(async move {
            if let Some(error) = channel_error.or(nick_error) {
                return Err(error.into());
            }
            let state = client.state();
            channel
                .check(state.isupport().inner())
                .and_then(|()| nick.check(state.isupport().inner()))
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
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
        })
    }

    /// Sets a channel ban and, only after confirming it, removes `nick` from the channel.
    ///
    /// The ban remains set when the kick is refused. Leaving or changing nickname during ban
    /// confirmation produces [`Error::TargetGone`] in [`crate::KickbanOutcome::kick`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unconfirmed`] without sending `KICK` when the ban cannot be proved. A ban
    /// refusal and the errors documented by [`Client::kick`] are returned before an outcome.
    pub fn kickban(
        &self,
        channel: impl crate::Target,
        nick: impl crate::Target,
        reason: impl AsRef<str>,
        ban: impl Into<crate::BanTarget>,
    ) -> crate::RequestFuture<crate::KickbanOutcome> {
        let client = self.clone();
        let (channel, channel_error) = self.target_input(channel);
        let (nick, nick_error) = self.target_input(nick);
        let state = self.state();
        let channel = state.channel_name(channel);
        let nick = state.nickname(nick);
        let reason = reason.as_ref().to_owned();
        let ban = ban.into();
        crate::RequestFuture::new(async move {
            if let Some(error) = channel_error.or(nick_error) {
                return Err(error.into());
            }
            let state = client.state();
            channel
                .check(state.isupport().inner())
                .and_then(|()| nick.check(state.isupport().inner()))
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            validate_kick_reason(&state, &reason)?;
            let ban = ban.resolve(&state)?;
            let already_set = crate::mode::tracked_ban(&state, &channel, &ban);
            let target_was_present = state
                .channel(&channel)
                .is_some_and(|joined| joined.member(&nick).is_some());
            drop(state);

            if !already_set {
                let response = client
                    .correlated(
                        Line::new("MODE").with_params([channel.as_ref(), "+b", ban.as_ref()]),
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
                        .is_none_or(|joined| joined.member(&nick).is_none())
                {
                    return Ok(crate::KickbanOutcome {
                        kick: Err(Error::TargetGone),
                    });
                }
            }

            Ok(crate::KickbanOutcome {
                kick: client.kick(&channel, &nick, &reason).await,
            })
        })
    }

    /// Invites `nick` to `channel`.
    ///
    /// # Errors
    ///
    /// Returns a typed server refusal, invalid-input error, timeout, or disconnect.
    pub fn invite(
        &self,
        channel: impl crate::Target,
        nick: impl crate::Target,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let (channel, channel_error) = self.target_input(channel);
        let (nick, nick_error) = self.target_input(nick);
        let state = self.state();
        let channel = state.channel_name(channel);
        let nick = state.nickname(nick);
        crate::RequestFuture::new(async move {
            if let Some(error) = channel_error.or(nick_error) {
                return Err(error.into());
            }
            let state = client.state();
            channel
                .check(state.isupport().inner())
                .and_then(|()| nick.check(state.isupport().inner()))
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
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
    pub fn set_nick(&self, nick: impl crate::Target) -> crate::RequestFuture<()> {
        let client = self.clone();
        let (nick, invalid_input) = self.target_input(nick);
        let nick = self.state().nickname(nick);
        crate::RequestFuture::new(async move {
            if let Some(error) = invalid_input {
                return Err(error.into());
            }
            nick.check(client.state().isupport().inner())
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
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
        channel: impl crate::Target,
        topic: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let (channel, invalid_input) = self.target_input(channel);
        self.change_topic(
            self.state().channel_name(channel),
            topic.into(),
            false,
            invalid_input,
        )
    }

    /// Clears the topic for `channel`.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Client::set_topic`].
    pub fn clear_topic(&self, channel: impl crate::Target) -> crate::RequestFuture<()> {
        let (channel, invalid_input) = self.target_input(channel);
        self.change_topic(
            self.state().channel_name(channel),
            String::new(),
            true,
            invalid_input,
        )
    }

    fn change_topic(
        &self,
        channel: crate::ChannelName,
        topic: String,
        clear: bool,
        invalid_input: Option<InvalidInput>,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            if let Some(error) = invalid_input {
                return Err(error.into());
            }
            if topic.is_empty() && !clear {
                return Err(InvalidInput::EmptyText.into());
            }
            channel
                .check(client.state().isupport().inner())
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            let response = client
                .correlated(
                    Line::new("TOPIC").with_params([channel.to_string(), topic]),
                    Some(ResponseMatcher::Topic(channel.to_string())),
                )
                .await?;
            crate::requests::empty(&response)
        })
    }

    /// Lists visible channels matching `filter`.
    ///
    /// At most 10,000 entries are retained.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::TryAgain`] or
    /// [`crate::Refusal::Other`] when refused. It can also return [`Error::InvalidInput`],
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub fn list(&self, filter: crate::ListFilter) -> crate::RequestFuture<Vec<crate::ListEntry>> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let message = match filter.parameter() {
                Some(parameter) => Line::new("LIST").with_params([parameter]),
                None => Line::new("LIST"),
            };
            let response = client
                .correlated(message, Some(ResponseMatcher::List))
                .await?;
            crate::requests::list(response, client.state().isupport().inner())
        })
    }

    /// Requests a clean disconnect and waits for the peer to close.
    ///
    /// The wait is limited to ten seconds. The driver still completes cleanly
    /// when that limit is reached. An I/O failure while waiting for closure
    /// also resolves to `Ok(())`. A timed-out acknowledgement is recorded in
    /// the final [`crate::Event::Disconnected`].
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
