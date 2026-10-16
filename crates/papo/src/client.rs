use std::{
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};

use papo_core::{HistoryQuery, Response, ResponseMatcher, TextKind};
use papo_proto::{Message, Typing, time};
use tokio::sync::{mpsc, oneshot};

use crate::{
    Error,
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
    pub(crate) connected: Arc<AtomicBool>,
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

    /// Starts configuring a client for `address`.
    pub fn builder(address: &str) -> crate::Builder {
        crate::Builder::new(address)
    }

    /// Reports whether an IRC transport is currently connected.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Borrows the latest immutable state snapshot for the duration of `read`.
    pub fn state<R>(&self, read: impl FnOnce(&State) -> R) -> R {
        let snapshot = self.snapshot();
        read(&snapshot)
    }

    /// Returns the latest immutable state snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Arc<State> {
        Arc::clone(
            &self
                .state
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Returns the client's current nickname.
    #[must_use]
    pub fn nick(&self) -> String {
        self.state(|state| state.nick().to_owned())
    }

    /// Reports whether a capability or its ratified alias is enabled.
    #[must_use]
    pub fn has_cap(&self, name: &str) -> bool {
        self.state(|state| state.has_cap(name))
    }

    /// Returns the latest accumulated server feature advertisement.
    #[must_use]
    pub fn isupport(&self) -> Arc<papo_proto::Isupport> {
        self.state(|state| Arc::clone(&state.isupport))
    }

    /// Returns the canonical form of an IRC nickname or channel name.
    #[must_use]
    pub fn casemap(&self, name: &str) -> String {
        self.state(|state| state.casemap(name))
    }

    /// Requests a channel rename.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn rename(&self, old: &str, new: &str, reason: &str) -> Result<(), Error> {
        let response = self
            .core_request(CoreRequest::Rename {
                old: old.into(),
                new: new.into(),
                reason: reason.into(),
            })
            .await?;
        if let Some(error) = response.channel_rename_error() {
            return Err(Error::ChannelRename(error));
        }
        crate::requests::empty(&response)
    }

    /// Sends a message attributed to a relayed nickname.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn relay(&self, channel: &str, nick: &str, text: &str) -> Result<(), Error> {
        let response = self
            .core_request(CoreRequest::Relay {
                channel: channel.into(),
                nick: nick.into(),
                text: text.into(),
            })
            .await?;
        if let Some(error) = response.relay_error() {
            return Err(Error::Relay(error));
        }
        crate::requests::empty(&response)
    }

    /// Validates and queues an IRC message for transmission.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Build`] if the message is invalid, or
    /// [`Error::Disconnected`] if the driver has stopped.
    pub async fn send(&self, message: Message) -> Result<(), Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::Send { message, result })
            .await
            .map_err(|_| Error::Disconnected)?;
        receiver.await.map_err(|_| Error::Disconnected)?
    }

    /// Sends an IRC message and waits for its labelled response.
    ///
    /// A server failure remains part of the successful [`Response`]. Use
    /// [`Response::error`] to inspect it.
    ///
    /// # Errors
    ///
    /// Returns an error if the message is invalid, the request limit is reached,
    /// the response times out, or the driver disconnects.
    pub async fn raw(&self, message: Message) -> Result<Response, Error> {
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
            .map_err(|_| Error::Disconnected)?;
        receiver.await.map_err(|_| Error::Disconnected)?
    }

    pub(crate) async fn core_request(&self, request: CoreRequest) -> Result<Response, Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::CoreRequest { request, result })
            .await
            .map_err(|_| Error::Disconnected)?;
        receiver.await.map_err(|_| Error::Disconnected)?
    }

    /// Sends an IRC message without waiting for a correlated response.
    ///
    /// # Errors
    ///
    /// Returns an error if the message is invalid or the driver has stopped.
    pub async fn raw_unlabeled(&self, message: Message) -> Result<(), Error> {
        self.send(message).await
    }

    /// Starts a chat history request for `target`.
    #[must_use]
    pub fn history(&self, target: &str) -> crate::History<'_> {
        crate::History {
            client: self,
            target: target.to_owned(),
        }
    }

    pub(crate) async fn history_page(
        &self,
        query: HistoryQuery,
    ) -> Result<crate::HistoryPage, Error> {
        let response = self.history_response(query).await?;
        let page = response.history.ok_or(Error::Disconnected)?;
        let messages = page
            .messages
            .into_iter()
            .filter_map(|item| {
                crate::MessageEvent::from_history(item.message, self.clone(), SystemTime::now())
                    .ok()
            })
            .collect();
        Ok(crate::HistoryPage {
            messages,
            end: page.end,
        })
    }

    async fn history_response(&self, query: HistoryQuery) -> Result<Response, Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::History { query, result })
            .await
            .map_err(|_| Error::Disconnected)?;
        let response = receiver.await.map_err(|_| Error::Disconnected)??;
        if let Some(error) = response.error() {
            return Err(Error::Server(error));
        }
        Ok(response)
    }

    /// Lists targets that received messages during an inclusive time range.
    ///
    /// # Errors
    ///
    /// Returns an error when history is unsupported or the request cannot complete.
    pub async fn history_targets(
        &self,
        since: SystemTime,
        until: SystemTime,
    ) -> Result<Vec<(String, SystemTime)>, Error> {
        self.history_response(HistoryQuery::Targets {
            from: since,
            to: until,
            limit: 1_000,
        })
        .await?
        .history_targets
        .ok_or(Error::Disconnected)
    }

    /// Sets the server read marker for `target`.
    ///
    /// # Errors
    ///
    /// Returns an error when read markers are unsupported or the request cannot complete.
    pub async fn mark_read(&self, target: &str, time: SystemTime) -> Result<(), Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::MarkRead {
                target: target.to_owned(),
                time,
                result,
            })
            .await
            .map_err(|_| Error::Disconnected)?;
        let response = receiver.await.map_err(|_| Error::Disconnected)??;
        crate::requests::empty(&response)
    }

    /// Queries the server read marker for `target`.
    ///
    /// # Errors
    ///
    /// Returns an error when read markers are unsupported or the query cannot complete.
    pub async fn read_marker(&self, target: &str) -> Result<Option<SystemTime>, Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::ReadMarker {
                target: target.to_owned(),
                result,
            })
            .await
            .map_err(|_| Error::Disconnected)?;
        receiver.await.map_err(|_| Error::Disconnected)?
    }

    /// Sends a private message to a user or channel.
    ///
    /// A labelled echo supplies its message identifier and optional server time. Both fields are
    /// `None` unless `echo-message` and `labeled-response` are enabled.
    ///
    /// # Errors
    ///
    /// Returns an error if the message is invalid or the driver has stopped.
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
    /// Returns an error if the text is empty or invalid, or the driver has stopped.
    pub async fn notice(&self, target: &str, text: &str) -> Result<Sent, Error> {
        self.send_text(TextKind::Notice, target, text, Vec::new())
            .await
    }

    /// Adds `emoji` as a reaction to `message`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when message tags, a message identifier, or an allowed
    /// reaction or reply tag is unavailable. Other errors indicate that sending failed.
    pub async fn react(&self, message: &crate::MessageEvent, emoji: &str) -> Result<(), Error> {
        self.reaction(message, emoji, "+draft/react").await
    }

    /// Removes `emoji` as a reaction to `message`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when message tags, a message identifier, or an allowed
    /// reaction or reply tag is unavailable. Other errors indicate that sending failed.
    pub async fn unreact(&self, message: &crate::MessageEvent, emoji: &str) -> Result<(), Error> {
        self.reaction(message, emoji, "+draft/unreact").await
    }

    async fn reaction(
        &self,
        message: &crate::MessageEvent,
        emoji: &str,
        reaction_tag: &'static str,
    ) -> Result<(), Error> {
        if !self.has_cap("message-tags") {
            return Err(Error::Unsupported("message-tags"));
        }
        let isupport = self.isupport();
        let denied = isupport.clienttagdeny();
        let reply_tag = event::allowed_alias(&denied, "+reply", "+draft/reply")
            .ok_or(Error::Unsupported("reply tag"))?;
        if denied.denies(reaction_tag) {
            return Err(Error::Unsupported("reaction tag"));
        }
        let msgid = message.msgid().ok_or(Error::Unsupported("message id"))?;
        self.send(
            Message::tagmsg(message.reply_target())
                .with_tag(reply_tag, msgid)
                .with_tag(reaction_tag, emoji),
        )
        .await
    }

    /// Sends a typing state for `target`.
    ///
    /// Repeated active states within three seconds are silently dropped. The call is also a
    /// no-op when message tags are unavailable or the server blocks the typing tag.
    ///
    /// # Errors
    ///
    /// Returns an error if the driver stops or the resulting message is invalid.
    pub async fn typing(&self, target: &str, state: Typing) -> Result<(), Error> {
        if !self.has_cap("message-tags") || self.isupport().clienttagdeny().denies("typing") {
            return Ok(());
        }
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::Typing {
                target: target.to_owned(),
                state,
                result,
            })
            .await
            .map_err(|_| Error::Disconnected)?;
        receiver.await.map_err(|_| Error::Disconnected)?
    }

    /// Redacts a message from a target's history.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when message redaction is unavailable, and
    /// [`Error::Server`] when the server rejects the labelled request.
    pub async fn redact(
        &self,
        target: &str,
        msgid: &str,
        reason: Option<&str>,
    ) -> Result<(), Error> {
        if !self.has_cap("draft/message-redaction") {
            return Err(Error::Unsupported("draft/message-redaction"));
        }
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
        if !self.is_connected() {
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
            .map_err(|_| Error::Disconnected)?;
        receiver.await.map_err(|_| Error::Disconnected)?
    }

    /// Requests information about a nickname.
    ///
    /// # Errors
    ///
    /// Returns an error when the server rejects the request or it cannot complete.
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
    /// Returns an error when the server rejects the request or it cannot complete.
    pub async fn who(&self, mask: &str) -> Result<Vec<crate::WhoEntry>, Error> {
        let isupport = self.isupport();
        let extended = isupport.whox();
        let message = if extended {
            Message::new("WHO", [mask, "%tcuhnfar"])
        } else {
            Message::new("WHO", [mask])
        };
        let response = self
            .correlated(message, Some(ResponseMatcher::Who(mask.to_owned())))
            .await?;
        crate::requests::who(response, &isupport, extended)
    }

    /// Joins a channel and waits for its topic and member list to synchronize.
    ///
    /// # Errors
    ///
    /// Returns an error when the server rejects the join or it cannot complete.
    pub async fn join(&self, channel: &str) -> Result<crate::ChannelInfo, Error> {
        let response = self
            .correlated(
                Message::join(channel),
                Some(ResponseMatcher::Join(channel.to_owned())),
            )
            .await?;
        if let Some(error) = response.error() {
            return Err(Error::Server(error));
        }
        if !self.state(|state| {
            state
                .channel(channel)
                .is_some_and(papo_core::Channel::is_synced)
        }) {
            let (result, receiver) = oneshot::channel();
            self.commands
                .send(Command::WaitChannelSync {
                    channel: channel.to_owned(),
                    result,
                })
                .await
                .map_err(|_| Error::Disconnected)?;
            receiver.await.map_err(|_| Error::Disconnected)??;
        }
        self.state(|state| {
            let joined = state.channel(channel).ok_or(Error::Disconnected)?;
            Ok(crate::ChannelInfo {
                name: joined.name().to_owned(),
                topic: joined.topic().map(str::to_owned),
                members_count: joined.members().count(),
            })
        })
    }

    /// Leaves a channel, optionally supplying a reason.
    ///
    /// # Errors
    ///
    /// Returns an error when the server rejects the part or it cannot complete.
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
    /// Returns an error when the server rejects the change or it cannot complete.
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
    /// Returns an error when the server rejects the request or it cannot complete.
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
    /// Returns an error when the server rejects the request or it cannot complete.
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
            .map_err(|_| Error::Disconnected)?;
        receiver.await.map_err(|_| Error::Disconnected)?
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Client").finish_non_exhaustive()
    }
}
