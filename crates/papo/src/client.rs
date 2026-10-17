use std::{
    sync::{Arc, RwLock},
    time::SystemTime,
};

use papo_core::{Response, ResponseMatcher, TextKind};
use papo_proto::{Message, Typing, time};
use tokio::sync::{mpsc, oneshot};

use crate::{
    Error, Feature, InvalidInput, Unsupported, UnsupportedReason,
    driver::{Command, CoreRequest},
    event,
    state::State,
};

#[cfg(test)]
use crate::driver::channels;
#[cfg(test)]
use papo_core::Session;

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
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Sent {
    /// Network-unique identifier from a labelled echo.
    pub msgid: Option<String>,
    /// Server time from a labelled echo.
    pub time: Option<SystemTime>,
}

impl Sent {
    pub(crate) fn from_response(response: &Response) -> Self {
        response.sent().map_or_else(Self::default, |sent| Self {
            msgid: Some(sent.msgid),
            time: sent.time.and_then(|value| time::parse(&value)),
        })
    }
}

impl Client {
    #[cfg(test)]
    pub(crate) fn disconnected_for_test() -> Self {
        channels(1, &Session::new(papo_core::Config::default())).client
    }

    #[cfg(all(test, feature = "bot"))]
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

    fn require_feature(state: &State, feature: Feature) -> Result<(), Error> {
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
    pub async fn reconnect(&self) -> Result<(), Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::Reconnect { result })
            .await
            .map_err(|_| Error::disconnected())?;
        receiver.await.map_err(|_| Error::disconnected())?
    }

    /// Validates and queues an IRC message for transmission.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if the message is invalid, or
    /// [`Error::Disconnected`] if the driver has stopped.
    pub async fn send(&self, message: Message) -> Result<(), Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::Send { message, result })
            .await
            .map_err(|_| Error::disconnected())?;
        receiver.await.map_err(|_| Error::disconnected())?
    }

    /// Sends an IRC message and waits for its labelled response.
    ///
    /// A server failure remains part of the successful [`Response`]. Use
    /// [`Response::error`] to inspect it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`Feature::LabeledResponse`] with reason
    /// [`UnsupportedReason::NotOffered`] or [`UnsupportedReason::DisabledByClient`]. It also
    /// returns [`Error::InvalidInput`], [`Error::RequestLimit`], [`Error::Timeout`], or
    /// [`Error::Disconnected`]. Server refusals remain in the returned [`Response`].
    pub async fn raw(&self, message: Message) -> Result<Response, Error> {
        Self::require_feature(&self.state(), Feature::LabeledResponse)?;
        self.correlated(message, None).await
    }

    async fn correlated(
        &self,
        message: Message,
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
            .map_err(|_| Error::disconnected())?;
        receiver.await.map_err(|_| Error::disconnected())?
    }

    pub(crate) async fn core_request(&self, request: CoreRequest) -> Result<Response, Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::CoreRequest { request, result })
            .await
            .map_err(|_| Error::disconnected())?;
        receiver.await.map_err(|_| Error::disconnected())?
    }

    /// Sends an IRC message without waiting for a correlated response.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if the message cannot be encoded, or
    /// [`Error::Disconnected`] if the driver has stopped.
    pub async fn raw_unlabeled(&self, message: Message) -> Result<(), Error> {
        self.send(message).await
    }

    /// Sends a private message to a user or channel.
    ///
    /// A labelled echo supplies its message identifier and optional server time. Both fields are
    /// `None` unless `echo-message` and `labeled-response` are enabled.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for empty text or an unencodable message,
    /// [`Error::RequestLimit`] when 256 labelled requests are pending, [`Error::Timeout`] while
    /// awaiting a response, or [`Error::Disconnected`] when the driver stops. A refusal returns
    /// [`Error::Server`] with [`crate::Refusal::CannotSendToChannel`],
    /// [`crate::Refusal::NoSuchNick`], [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::NoTextToSend`], [`crate::Refusal::InputTooLong`],
    /// [`crate::Refusal::TryAgain`], or [`crate::Refusal::Other`].
    pub async fn privmsg(&self, target: &str, text: &str) -> Result<Sent, Error> {
        self.send_text(TextKind::Privmsg, target, text, Vec::new())
            .await
    }

    /// Sends a notice to a user or channel.
    ///
    /// A labelled echo supplies its message identifier and optional server time. Both fields are
    /// `None` unless `echo-message` and `labeled-response` are enabled.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for empty text or an unencodable message,
    /// [`Error::RequestLimit`] when 256 labelled requests are pending, [`Error::Timeout`] while
    /// awaiting a response, or [`Error::Disconnected`] when the driver stops. A refusal returns
    /// [`Error::Server`] with [`crate::Refusal::CannotSendToChannel`],
    /// [`crate::Refusal::NoSuchNick`], [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::NoTextToSend`], [`crate::Refusal::InputTooLong`],
    /// [`crate::Refusal::TryAgain`], or [`crate::Refusal::Other`].
    pub async fn notice(&self, target: &str, text: &str) -> Result<Sent, Error> {
        self.send_text(TextKind::Notice, target, text, Vec::new())
            .await
    }

    /// Adds `emoji` as a reaction to `message`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`Feature::Reactions`] with reason
    /// [`UnsupportedReason::NotOffered`], [`UnsupportedReason::DisabledByClient`], or
    /// [`UnsupportedReason::TagDenied`]. A missing message identifier returns
    /// [`Error::InvalidInput`]; encoding or driver failure returns [`Error::InvalidInput`] or
    /// [`Error::Disconnected`].
    pub async fn react(&self, message: &crate::MessageEvent, emoji: &str) -> Result<(), Error> {
        self.reaction(message, emoji, "+draft/react").await
    }

    /// Removes `emoji` as a reaction to `message`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`Feature::Reactions`] with reason
    /// [`UnsupportedReason::NotOffered`], [`UnsupportedReason::DisabledByClient`], or
    /// [`UnsupportedReason::TagDenied`]. A missing message identifier returns
    /// [`Error::InvalidInput`]; encoding or driver failure returns [`Error::InvalidInput`] or
    /// [`Error::Disconnected`].
    pub async fn unreact(&self, message: &crate::MessageEvent, emoji: &str) -> Result<(), Error> {
        self.reaction(message, emoji, "+draft/unreact").await
    }

    async fn reaction(
        &self,
        message: &crate::MessageEvent,
        emoji: &str,
        reaction_tag: &'static str,
    ) -> Result<(), Error> {
        let state = self.state();
        Self::require_feature(&state, Feature::Reactions)?;
        let denied = state.isupport().clienttagdeny();
        let reply_tag = event::allowed_alias(&denied, "+reply", "+draft/reply")
            .expect("feature preflight requires an allowed reply tag");
        if denied.denies(reaction_tag) {
            return Err(Error::Unsupported(Unsupported::new(
                Feature::Reactions,
                UnsupportedReason::TagDenied,
            )));
        }
        let msgid = message
            .msgid()
            .ok_or(Error::InvalidInput(InvalidInput::MessageReference))?;
        self.send(
            Message::tagmsg(message.reply_target())
                .with_tag(reply_tag, msgid)
                .with_tag(reaction_tag, emoji),
        )
        .await
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
    pub async fn typing(&self, target: &str, state: Typing) -> Result<(), Error> {
        let snapshot = self.state();
        Self::require_feature(&snapshot, Feature::Typing)?;
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::Typing {
                target: target.to_owned(),
                state,
                result,
            })
            .await
            .map_err(|_| Error::disconnected())?;
        receiver.await.map_err(|_| Error::disconnected())?
    }

    /// Redacts a message from a target's history.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`Feature::Redaction`] with reason
    /// [`UnsupportedReason::NotOffered`] or [`UnsupportedReason::DisabledByClient`]. A refusal
    /// returns [`Error::Server`], currently classified as [`crate::Refusal::Other`]. It can also
    /// return [`Error::InvalidInput`], [`Error::RequestLimit`], [`Error::Timeout`], or
    /// [`Error::Disconnected`].
    pub async fn redact(
        &self,
        target: &str,
        msgid: &str,
        reason: Option<&str>,
    ) -> Result<(), Error> {
        Self::require_feature(&self.state(), Feature::Redaction)?;
        let mut params = vec![target, msgid];
        params.extend(reason);
        let response = self
            .correlated(Message::new("REDACT", params), None)
            .await?;
        crate::requests::empty(&response)
    }

    pub(crate) async fn send_text(
        &self,
        kind: TextKind,
        target: &str,
        text: &str,
        tags: Vec<(String, String)>,
    ) -> Result<Sent, Error> {
        if !self.state().is_connected() {
            let mut message = match kind {
                TextKind::Privmsg => Message::privmsg(target, text),
                TextKind::Notice => Message::notice(target, text),
            };
            for (key, value) in tags {
                message = message.with_tag(key, value);
            }
            self.send(message).await?;
            return Ok(Sent::default());
        }
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::SendText {
                kind,
                target: target.to_owned(),
                text: text.to_owned(),
                tags,
                result,
            })
            .await
            .map_err(|_| Error::disconnected())?;
        receiver.await.map_err(|_| Error::disconnected())?
    }

    /// Requests information about a nickname.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::NoSuchNick`],
    /// [`crate::Refusal::NoSuchServer`], [`crate::Refusal::TryAgain`], or
    /// [`crate::Refusal::Other`] when refused. It can also return [`Error::InvalidInput`],
    /// [`Error::RequestLimit`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub async fn whois(&self, nick: &str) -> Result<crate::Whois, Error> {
        let response = self
            .correlated(
                Message::new("WHOIS", [nick]),
                Some(ResponseMatcher::Whois(nick.to_owned())),
            )
            .await?;
        crate::requests::whois(response, nick)
    }

    /// Requests the users matching a channel or mask.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::TryAgain`] or
    /// [`crate::Refusal::Other`] when refused. It can also return [`Error::InvalidInput`],
    /// [`Error::RequestLimit`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub async fn who(&self, mask: &str) -> Result<Vec<crate::WhoEntry>, Error> {
        let state = self.state();
        let extended = state.isupport().whox();
        let message = if extended {
            Message::new("WHO", [mask, "%tcuhnfar"])
        } else {
            Message::new("WHO", [mask])
        };
        let response = self
            .correlated(message, Some(ResponseMatcher::Who(mask.to_owned())))
            .await?;
        crate::requests::who(response, state.isupport(), extended)
    }

    /// Joins a channel and waits for its topic and member list to synchronize.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::TooManyChannels`], [`crate::Refusal::NickUnavailable`],
    /// [`crate::Refusal::ChannelIsFull`], [`crate::Refusal::InviteOnlyChannel`],
    /// [`crate::Refusal::BannedFromChannel`], [`crate::Refusal::BadChannelKey`],
    /// [`crate::Refusal::BadChannelMask`], [`crate::Refusal::SecureOnlyChannel`],
    /// [`crate::Refusal::TryAgain`], or [`crate::Refusal::Other`] when refused. It can also
    /// return [`Error::InvalidInput`], [`Error::RequestLimit`], [`Error::Timeout`], or
    /// [`Error::Disconnected`].
    pub async fn join(&self, channel: &str) -> Result<crate::ChannelInfo, Error> {
        let response = self
            .correlated(
                Message::join(channel),
                Some(ResponseMatcher::Join(channel.to_owned())),
            )
            .await?;
        if let Some(error) = response.error() {
            return Err(Error::server(error));
        }
        if self.state().channel(channel).is_none() {
            return Err(Error::disconnected());
        }
        if !self
            .state()
            .channel(channel)
            .is_some_and(papo_core::Channel::is_synced)
        {
            let (result, receiver) = oneshot::channel();
            self.commands
                .send(Command::WaitChannelSync {
                    channel: channel.to_owned(),
                    result,
                })
                .await
                .map_err(|_| Error::disconnected())?;
            receiver.await.map_err(|_| Error::disconnected())??;
        }
        let state = self.state();
        let joined = state.channel(channel).ok_or_else(Error::disconnected)?;
        Ok(crate::ChannelInfo {
            name: joined.name().as_ref().to_owned(),
            topic: joined.topic().map(str::to_owned),
            members_count: joined.members().count(),
        })
    }

    /// Leaves a channel, optionally supplying a reason.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::NotOnChannel`], [`crate::Refusal::TryAgain`], or
    /// [`crate::Refusal::Other`] when refused. It can also return [`Error::InvalidInput`],
    /// [`Error::RequestLimit`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub async fn part(&self, channel: &str, reason: Option<&str>) -> Result<(), Error> {
        let mut params = vec![channel];
        params.extend(reason);
        let response = self
            .correlated(
                Message::new("PART", params),
                Some(ResponseMatcher::Part(channel.to_owned())),
            )
            .await?;
        crate::requests::empty(&response)
    }

    /// Applies a mode string and its arguments to a channel or user.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::NoSuchNick`],
    /// [`crate::Refusal::NoSuchChannel`], [`crate::Refusal::NotOnChannel`],
    /// [`crate::Refusal::NoPrivileges`], [`crate::Refusal::ChannelOperatorPrivilegesNeeded`],
    /// [`crate::Refusal::UnknownMode`], [`crate::Refusal::UnknownUserMode`],
    /// [`crate::Refusal::UsersDontMatch`], [`crate::Refusal::InvalidModeParameter`],
    /// [`crate::Refusal::TryAgain`], or [`crate::Refusal::Other`] when refused. It can also
    /// return [`Error::InvalidInput`], [`Error::RequestLimit`], [`Error::Timeout`], or
    /// [`Error::Disconnected`].
    pub async fn mode<I, S>(&self, target: &str, modes: &str, args: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut params = vec![target.to_owned(), modes.to_owned()];
        params.extend(args.into_iter().map(|arg| arg.as_ref().to_owned()));
        let response = self
            .correlated(
                Message::new("MODE", params),
                Some(ResponseMatcher::Mode(target.to_owned())),
            )
            .await?;
        crate::requests::empty(&response)
    }

    /// Sets a channel topic, or queries it when `text` is `None`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::NotOnChannel`], [`crate::Refusal::ChannelOperatorPrivilegesNeeded`],
    /// [`crate::Refusal::TryAgain`], or [`crate::Refusal::Other`] when refused. It can also
    /// return [`Error::InvalidInput`], [`Error::RequestLimit`], [`Error::Timeout`], or
    /// [`Error::Disconnected`].
    pub async fn topic(&self, channel: &str, text: Option<&str>) -> Result<Option<String>, Error> {
        let mut params = vec![channel];
        params.extend(text);
        let response = self
            .correlated(
                Message::new("TOPIC", params),
                Some(ResponseMatcher::Topic(channel.to_owned())),
            )
            .await?;
        crate::requests::topic(&response)
    }

    /// Lists visible channels matching `filter`.
    ///
    /// At most 10,000 entries are retained.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Server`] with [`crate::Refusal::TryAgain`] or
    /// [`crate::Refusal::Other`] when refused. It can also return [`Error::InvalidInput`],
    /// [`Error::RequestLimit`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub async fn list(&self, filter: crate::ListFilter) -> Result<Vec<crate::ListEntry>, Error> {
        let message = match filter.parameter() {
            Some(parameter) => Message::new("LIST", [parameter]),
            None => Message::new("LIST", Vec::<String>::new()),
        };
        let response = self
            .correlated(message, Some(ResponseMatcher::List))
            .await?;
        crate::requests::list(response)
    }

    /// Requests a clean disconnect and waits for the peer to close.
    ///
    /// The wait is limited to ten seconds. The driver still completes cleanly
    /// when that limit is reached. An I/O failure while waiting for closure
    /// also resolves to `Ok(())`; its cause is available in the `reason`
    /// field of [`crate::Event::Disconnected`].
    ///
    /// # Errors
    ///
    /// Returns an error if the QUIT message is invalid, the driver has already
    /// stopped, or the peer does not close within ten seconds.
    pub async fn quit(&self, reason: Option<&str>) -> Result<(), Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::Quit {
                reason: reason.map(str::to_owned),
                result,
            })
            .await
            .map_err(|_| Error::disconnected())?;
        receiver.await.map_err(|_| Error::disconnected())?
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Client").finish_non_exhaustive()
    }
}
