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

#[cfg(all(test, not(browser)))]
use crate::driver::channels;
#[cfg(all(test, not(browser)))]
use papo_core::Session;

static NEXT_PING: AtomicU64 = AtomicU64::new(0);

/// A cheap, cloneable handle to a running IRC client.
///
/// Typed requests use server labels when available and otherwise match protocol
/// reply boundaries. Concurrent identical unlabelled requests resolve in FIFO order.
#[derive(Clone)]
pub struct Client {
    pub(crate) commands: mpsc::Sender<Command>,
    pub(crate) state: Arc<RwLock<Arc<State>>>,
}

/// Server-assigned metadata for a sent chat message.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Sent {
    /// Target to which the message was sent.
    pub target: crate::Target,
    /// Server-assigned identifier from a labelled echo.
    pub id: Option<crate::MessageId>,
    /// Server time from a labelled echo.
    pub time: Option<SystemTime>,
    /// Compatibility fallbacks applied while sending.
    pub fallbacks: Vec<Fallback>,
}

/// A compatibility fallback applied to an outgoing chat message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
pub struct SendMessage {
    client: Client,
    kind: TextKind,
    target: crate::Target,
    text: String,
    reply_to: Option<crate::MessageId>,
    channel_context: Option<crate::ChannelName>,
    tags: Vec<(String, String)>,
    invalid_input: Option<InvalidInput>,
}

/// Builds a channel join request.
#[derive(Debug)]
pub struct JoinChannel {
    client: Client,
    channel: crate::ChannelName,
    key: Option<crate::Secret>,
}

impl JoinChannel {
    /// Supplies the channel key without exposing it through `Debug` output.
    #[must_use]
    pub fn key(mut self, key: impl Into<crate::Secret>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl IntoFuture for JoinChannel {
    type Output = Result<(), Error>;
    type IntoFuture = crate::RequestFuture<()>;

    fn into_future(self) -> Self::IntoFuture {
        crate::RequestFuture::new(
            async move { self.client.join_channel(self.channel, self.key).await },
        )
    }
}

impl SendMessage {
    /// Marks this message as a reply to `message` when replies are available.
    #[must_use]
    pub fn reply_to(mut self, message: &crate::MessageId) -> Self {
        self.reply_to = Some(message.clone());
        self
    }

    /// Supplies channel context for a private message.
    #[must_use]
    pub fn channel_context(mut self, channel: impl AsRef<str>) -> Self {
        self.channel_context = Some(self.client.state().channel_name(channel.as_ref()));
        self
    }

    /// Adds a client-only message tag.
    ///
    /// The reply, reaction, and channel-context keys in [`crate::tags::keys`] are managed by
    /// typed methods; setting them here can conflict with those methods.
    #[must_use]
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
            self.target
                .check(state.isupport().inner())
                .map_err(|reason| InvalidInput::invalid_name(self.target.to_string(), reason))?;
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
            if state.has_cap(crate::cap::MESSAGE_TAGS) {
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
                .send_text(self.kind, self.target.as_ref(), &self.text, tags, fallbacks)
                .await
        })
    }
}

impl Sent {
    pub(crate) fn without_echo(target: &str, fallbacks: Vec<Fallback>) -> Self {
        Self {
            target: target.into(),
            id: None,
            time: None,
            fallbacks,
        }
    }

    pub(crate) fn from_response(
        response: &Response,
        target: &str,
        fallbacks: Vec<Fallback>,
    ) -> Self {
        match response.sent() {
            Some(sent) => Self {
                target: target.into(),
                id: Some(sent.msgid.into()),
                time: sent.time.and_then(|value| time::parse(&value)),
                fallbacks,
            },
            None => Self::without_echo(target, fallbacks),
        }
    }
}

impl Client {
    #[cfg(all(test, not(browser)))]
    pub(crate) fn disconnected_for_test() -> Self {
        channels(1, &Session::new(papo_core::Config::default())).client
    }

    #[cfg(all(test, not(browser), feature = "bot"))]
    pub(crate) fn from_session_for_test(session: &Session) -> Self {
        channels(1, session).client
    }

    /// Starts configuring a client for `endpoint`.
    ///
    /// A string is a hostname for TLS on port 6697. Use [`crate::Endpoint`]
    /// for another port, plaintext, or WebSocket transport.
    ///
    /// ```
    /// let builder = papo::Client::builder("irc.example.net");
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
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] when the line cannot be represented on
    /// the wire, or [`Error::Disconnected`] if the driver stops before writing it.
    pub fn send_raw(&self, line: impl Into<Line>) -> crate::RequestFuture<()> {
        let client = self.clone();
        let line = line.into();
        crate::RequestFuture::new(async move { client.send_message(line).await })
    }

    /// Sends one IRC protocol line and waits for its labelled response.
    ///
    /// Response lines are returned here rather than emitted by the event
    /// stream. Their state changes are still applied before this future
    /// completes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when `labeled-response` is unavailable;
    /// raw requests have no unlabelled fallback. Also returns an error when the
    /// line cannot be represented on the wire, the request limit is reached,
    /// the response times out, or the driver stops.
    pub fn request_raw(&self, line: impl Into<Line>) -> crate::RequestFuture<Vec<Line>> {
        let client = self.clone();
        let line = line.into();
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

    pub(crate) async fn core_request(&self, request: CoreRequest) -> Result<Response, Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::CoreRequest { request, result })
            .await
            .map_err(|_| Error::connection_lost())?;
        receiver.await.map_err(|_| Error::connection_lost())?
    }

    /// Sends a private message to a channel or directly to a user.
    ///
    /// A labelled echo supplies its message identifier and optional server time. Both fields are
    /// `None` unless `echo-message` and `labeled-response` are enabled.
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
    pub fn privmsg(&self, target: impl AsRef<str>, text: impl Into<String>) -> SendMessage {
        SendMessage {
            client: self.clone(),
            kind: TextKind::Privmsg,
            target: crate::Target::from(target.as_ref()),
            text: text.into(),
            reply_to: None,
            channel_context: None,
            tags: Vec::new(),
            invalid_input: None,
        }
    }

    /// Sends a notice to a user or channel.
    ///
    /// A labelled echo supplies its message identifier and optional server time. Both fields are
    /// `None` unless `echo-message` and `labeled-response` are enabled.
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
    pub fn notice(&self, target: impl AsRef<str>, text: impl Into<String>) -> SendMessage {
        SendMessage {
            client: self.clone(),
            kind: TextKind::Notice,
            target: crate::Target::from(target.as_ref()),
            text: text.into(),
            reply_to: None,
            channel_context: None,
            tags: Vec::new(),
            invalid_input: None,
        }
    }

    /// Sends a CTCP ACTION to a user or channel.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Client::privmsg`].
    pub fn action(&self, target: impl AsRef<str>, text: impl Into<String>) -> SendMessage {
        let text = text.into();
        if let Some(action) = Ctcp::action(&text) {
            self.privmsg(target, action)
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
    /// # Errors
    ///
    /// Returns an error if the text is empty or invalid, or the driver has stopped.
    pub fn reply(&self, message: &crate::Message, text: impl Into<String>) -> SendMessage {
        let mut request = self.privmsg(message.reply_target(), text);
        if let Some(id) = message.id() {
            request = request.reply_to(&id.into());
        }
        if message.is_private()
            && let Some(context) = message.channel_context()
        {
            request = request.channel_context(self.state().casemapping().channel(context));
        }
        request
    }

    #[cfg(feature = "bot")]
    pub(crate) async fn private_notice(
        &self,
        message: &crate::Message,
        text: &str,
    ) -> Result<(), Error> {
        let Some(target) = message.sender_nick() else {
            return Ok(());
        };
        let mut request = self.notice(target, text);
        if let Some(id) = message.id() {
            request = request.reply_to(&id.into());
        }
        let context = if message.is_private() {
            message.channel_context()
        } else {
            Some(message.target())
        };
        if let Some(context) = context {
            request = request.channel_context(self.state().casemapping().channel(context));
        }
        request.await.map(drop)
    }

    /// Adds `emoji` as a reaction to `message`.
    ///
    /// A reaction is usually an emoji but may be any nonempty text. Reactions are compared
    /// exactly, without case folding or Unicode normalization.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`Feature::Reactions`] with reason
    /// [`UnsupportedReason::NotOffered`], [`UnsupportedReason::DisabledByClient`], or
    /// [`UnsupportedReason::TagDenied`]. Encoding or driver failure returns
    /// [`Error::InvalidInput`] or [`Error::Disconnected`].
    pub fn react(
        &self,
        target: impl AsRef<str>,
        message: impl Into<crate::MessageId>,
        emoji: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        self.reaction(
            crate::Target::from(target.as_ref()),
            message.into(),
            emoji.into(),
            "+draft/react",
        )
    }

    /// Removes `emoji` as a reaction to `message`.
    ///
    /// `emoji` must exactly match the added reaction; papo performs no Unicode normalization.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`Feature::Reactions`] with reason
    /// [`UnsupportedReason::NotOffered`], [`UnsupportedReason::DisabledByClient`], or
    /// [`UnsupportedReason::TagDenied`]. Encoding or driver failure returns
    /// [`Error::InvalidInput`] or [`Error::Disconnected`].
    pub fn unreact(
        &self,
        target: impl AsRef<str>,
        message: impl Into<crate::MessageId>,
        emoji: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        self.reaction(
            crate::Target::from(target.as_ref()),
            message.into(),
            emoji.into(),
            "+draft/unreact",
        )
    }

    /// Adds `emoji` as a reaction to `message` using its routing target.
    ///
    /// A reaction is usually an emoji but may be any nonempty text and is compared exactly.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] when the message has no identifier, or the same errors as
    /// [`Client::react`].
    pub fn react_to(
        &self,
        message: &crate::Message,
        emoji: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let Some(id) = message.id() else {
            return crate::RequestFuture::new(async {
                Err(InvalidInput::UnusableMessageReference.into())
            });
        };
        self.react(message.target(), id, emoji)
    }

    fn reaction(
        &self,
        target: crate::Target,
        message: crate::MessageId,
        emoji: String,
        reaction_tag: &'static str,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
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
            Self::require_feature(&state, Feature::Reactions)?;
            target
                .check(state.isupport().inner())
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
            let message = Line::tagmsg(target.as_ref())
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

    /// Sends a typing state for `target`.
    ///
    /// Repeated active states within three seconds are silently dropped.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`Feature::Typing`] with reason
    /// [`UnsupportedReason::NotOffered`], [`UnsupportedReason::DisabledByClient`], or
    /// [`UnsupportedReason::TagDenied`]. Encoding or driver failure returns
    /// [`Error::InvalidInput`] or [`Error::Disconnected`].
    pub fn typing(&self, target: impl AsRef<str>, state: Typing) -> crate::RequestFuture<()> {
        let client = self.clone();
        let target = crate::Target::from(target.as_ref());
        crate::RequestFuture::new(async move {
            let snapshot = client.state();
            Self::require_feature(&snapshot, Feature::Typing)?;
            target
                .check(snapshot.isupport().inner())
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            let (result, receiver) = oneshot::channel();
            client
                .commands
                .send(Command::Typing {
                    target: target.into_string(),
                    state,
                    result,
                })
                .await
                .map_err(|_| Error::connection_lost())?;
            receiver.await.map_err(|_| Error::connection_lost())?
        })
    }

    /// Redacts a message from a target's history.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`Feature::Redaction`] with reason
    /// [`UnsupportedReason::NotOffered`] or [`UnsupportedReason::DisabledByClient`]. A refusal
    /// returns [`Error::Server`], currently classified as [`crate::Refusal::Other`]. It can also
    /// return [`Error::InvalidInput`], [`Error::TooManyPendingRequests`], [`Error::Timeout`], or
    /// [`Error::Disconnected`].
    pub fn redact(
        &self,
        target: impl AsRef<str>,
        message: impl Into<crate::MessageId>,
        reason: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let target = crate::Target::from(target.as_ref());
        let message = message.into();
        let reason = reason.into();
        crate::RequestFuture::new(async move {
            let state = client.state();
            Self::require_feature(&state, Feature::Redaction)?;
            target
                .check(state.isupport().inner())
                .and_then(|()| message.check())
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            let mut params = vec![target.to_string(), message.to_string()];
            if !reason.is_empty() {
                params.push(reason);
            }
            let response = client.correlated(Line::new("REDACT", params), None).await?;
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
    ) -> Result<Sent, Error> {
        if !self.state().is_connected() {
            let mut message = match kind {
                TextKind::Privmsg => Line::privmsg(target, text),
                TextKind::Notice => Line::notice(target, text),
            };
            for (key, value) in tags {
                message = message.with_tag(key, value);
            }
            self.send_message(message).await?;
            return Ok(Sent::without_echo(target, fallbacks));
        }
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::SendText {
                kind,
                target: target.to_owned(),
                text: text.to_owned(),
                tags,
                fallbacks,
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
    pub fn whois(&self, nick: impl AsRef<str>) -> crate::RequestFuture<crate::Whois> {
        let client = self.clone();
        let nick = self.state().nickname(nick.as_ref());
        crate::RequestFuture::new(async move {
            nick.check(client.state().isupport().inner())
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            let response = client
                .correlated(
                    Line::new("WHOIS", [nick.to_string()]),
                    Some(ResponseMatcher::Whois(nick.to_string())),
                )
                .await?;
            crate::requests::whois(response, &nick, client.state().isupport().inner())
        })
    }

    /// Requests the users matching a channel or mask.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::TryAgain`] or
    /// [`crate::Refusal::Other`] when refused. It can also return [`Error::InvalidInput`],
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub fn who(&self, mask: impl AsRef<str>) -> crate::RequestFuture<Vec<crate::WhoEntry>> {
        let client = self.clone();
        let mask = self.state().mask(mask.as_ref());
        crate::RequestFuture::new(async move {
            mask.check()
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            let state = client.state();
            let extended = state.isupport().whox();
            let message = if extended {
                Line::new("WHO", [mask.to_string(), "%tcuhnfar".into()])
            } else {
                Line::new("WHO", [mask.to_string()])
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
    pub fn join(&self, channel: impl AsRef<str>) -> JoinChannel {
        JoinChannel {
            client: self.clone(),
            channel: self.state().channel_name(channel.as_ref()),
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

    /// Leaves a channel, omitting an empty `reason` from the wire.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::NotOnChannel`], [`crate::Refusal::TryAgain`], or
    /// [`crate::Refusal::Other`] when refused. It can also return [`Error::InvalidInput`],
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub fn part(
        &self,
        channel: impl AsRef<str>,
        reason: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let channel = self.state().channel_name(channel.as_ref());
        let reason = reason.into();
        crate::RequestFuture::new(async move {
            channel
                .check(client.state().isupport().inner())
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            let mut params = vec![channel.to_string()];
            if !reason.is_empty() {
                params.push(reason);
            }
            let response = client
                .correlated(
                    Line::new("PART", params),
                    Some(ResponseMatcher::Part(channel.to_string())),
                )
                .await?;
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
        channel: impl AsRef<str>,
        nick: impl AsRef<str>,
        reason: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let state = self.state();
        let channel = state.channel_name(channel.as_ref());
        let nick = state.nickname(nick.as_ref());
        let reason = reason.into();
        crate::RequestFuture::new(async move {
            let state = client.state();
            channel
                .check(state.isupport().inner())
                .and_then(|()| nick.check(state.isupport().inner()))
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            let mut params = vec![channel.to_string(), nick.to_string()];
            if !reason.is_empty() {
                params.push(reason);
            }
            let response = client
                .correlated(
                    Line::new("KICK", params),
                    Some(ResponseMatcher::Kick {
                        channel: channel.to_string(),
                        nick: nick.to_string(),
                    }),
                )
                .await?;
            crate::requests::empty(&response)
        })
    }

    /// Invites `nick` to `channel`.
    ///
    /// # Errors
    ///
    /// Returns a typed server refusal, invalid-input error, timeout, or disconnect.
    pub fn invite(
        &self,
        channel: impl AsRef<str>,
        nick: impl AsRef<str>,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let state = self.state();
        let channel = state.channel_name(channel.as_ref());
        let nick = state.nickname(nick.as_ref());
        crate::RequestFuture::new(async move {
            let state = client.state();
            channel
                .check(state.isupport().inner())
                .and_then(|()| nick.check(state.isupport().inner()))
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            let response = client
                .correlated(
                    Line::new("INVITE", [nick.to_string(), channel.to_string()]),
                    Some(ResponseMatcher::Invite {
                        channel: channel.to_string(),
                        nick: nick.to_string(),
                    }),
                )
                .await?;
            crate::requests::empty(&response)
        })
    }

    /// Changes this client's nickname.
    ///
    /// # Errors
    ///
    /// Returns a typed server refusal, invalid-input error, timeout, or disconnect.
    pub fn set_nick(&self, nick: impl AsRef<str>) -> crate::RequestFuture<()> {
        let client = self.clone();
        let nick = self.state().nickname(nick.as_ref());
        crate::RequestFuture::new(async move {
            nick.check(client.state().isupport().inner())
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            let response = client
                .correlated(
                    Line::new("NICK", [nick.to_string()]),
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
                .correlated(Line::new("AWAY", [text]), Some(ResponseMatcher::Away(true)))
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
                .correlated(
                    Line::new("AWAY", Vec::<String>::new()),
                    Some(ResponseMatcher::Away(false)),
                )
                .await?;
            crate::requests::empty(&response)
        })
    }

    /// Changes this client's realname through the `setname` capability.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when `setname` is unavailable, or a typed server refusal,
    /// invalid-input error, timeout, or disconnect.
    pub fn set_realname(&self, realname: impl Into<String>) -> crate::RequestFuture<()> {
        let client = self.clone();
        let realname = realname.into();
        crate::RequestFuture::new(async move {
            Self::require_feature(&client.state(), Feature::SetName)?;
            let response = client
                .correlated(
                    Line::new("SETNAME", [realname.clone()]),
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
            let password = password.expose().ok_or(InvalidInput::BinarySecret)?;
            let response = client
                .correlated(
                    Line::new("OPER", [name, password.to_owned()]),
                    Some(ResponseMatcher::Oper),
                )
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
                    Line::new("PING", [token.clone()]),
                    Some(ResponseMatcher::Ping(token)),
                )
                .await?;
            crate::requests::empty(&response)?;
            Ok(crate::rt::now_mono().saturating_duration_since(started))
        })
    }

    /// Sets the topic for `channel`.
    ///
    /// An empty `topic` clears the topic, the same as [`Client::clear_topic`].
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
        channel: impl AsRef<str>,
        topic: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        self.change_topic(self.state().channel_name(channel.as_ref()), topic.into())
    }

    /// Clears the topic for `channel`.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Client::set_topic`].
    pub fn clear_topic(&self, channel: impl AsRef<str>) -> crate::RequestFuture<()> {
        self.change_topic(self.state().channel_name(channel.as_ref()), String::new())
    }

    fn change_topic(&self, channel: crate::ChannelName, topic: String) -> crate::RequestFuture<()> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            channel
                .check(client.state().isupport().inner())
                .map_err(|error| InvalidInput::InvalidMessage(error.to_string()))?;
            let response = client
                .correlated(
                    Line::new("TOPIC", [channel.to_string(), topic]),
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
                Some(parameter) => Line::new("LIST", [parameter]),
                None => Line::new("LIST", Vec::<String>::new()),
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
                Line::new("QUIT", Vec::<String>::new())
            } else {
                Line::new("QUIT", [reason])
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

impl std::fmt::Debug for Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Client").finish_non_exhaustive()
    }
}
