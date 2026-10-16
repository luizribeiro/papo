use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    future::Future,
    io,
    ops::ControlFlow,
    pin::Pin,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    task,
    time::{Duration, Instant, SystemTime},
};

use papo_core::{
    Connection, HistoryQuery, RequestError, RequestId, Response, ResponseMatcher, Session,
    TextKind,
    sts::{ConnectionSecurity, StoredPolicy, StsAction, StsStore, reschedule_on_disconnect},
};
use papo_proto::{Message, Typing, time};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

use crate::{
    AddressError, Error, Event, Reconnect,
    address::{Protocol, ServerAddress},
    event,
    state::State,
    transport::{self, TlsConfig},
};

pub(crate) const DEFAULT_CHANNEL_CAPACITY: usize = 1024;
const QUIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const STABLE_CONNECTION: Duration = Duration::from_secs(60);

pub(crate) trait ConnectionIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ConnectionIo for T {}
type BoxConnection = Box<dyn ConnectionIo>;
type ConnectFuture = Pin<Box<dyn Future<Output = Result<BoxConnection, Error>> + Send>>;
pub(crate) type Connector = Arc<dyn Fn(ConnectionPlan) -> ConnectFuture + Send + Sync>;
type BackoffEntropy = Box<dyn FnMut() -> u64 + Send>;
type ReadMarkerSender = oneshot::Sender<Result<Option<SystemTime>, Error>>;
type ReadMarkerWaiters = Vec<(String, ReadMarkerSender)>;

pub(crate) enum CoreRequest {
    Register {
        account: Option<String>,
        email: Option<String>,
        password: papo_core::Secret,
    },
    Verify {
        account: String,
        code: String,
    },
    PersistenceGet,
    PersistenceSet(papo_core::PersistenceSetting),
    Metadata(papo_core::MetadataCommand),
}

impl CoreRequest {
    fn start(self, session: &mut Session) -> Result<RequestId, RequestError> {
        let before = session.next_request_id();
        match self {
            Self::Register {
                account,
                email,
                password,
            } => {
                session.register(now(), account.as_deref(), email.as_deref(), password)?;
            }
            Self::Verify { account, code } => session.verify(now(), &account, &code)?,
            Self::PersistenceGet => session.persistence_get(now())?,
            Self::PersistenceSet(setting) => session.persistence_set(now(), setting)?,
            Self::Metadata(command) => return session.metadata(now(), command),
        }
        Ok(before)
    }
}

struct Catchup {
    target: String,
    messages: Vec<Message>,
    seen: HashSet<String>,
}

enum Command {
    Send {
        message: Message,
        result: oneshot::Sender<Result<(), Error>>,
    },
    Request {
        message: Message,
        matcher: Option<ResponseMatcher>,
        result: oneshot::Sender<Result<Response, Error>>,
    },
    CoreRequest {
        request: CoreRequest,
        result: oneshot::Sender<Result<Response, Error>>,
    },
    History {
        query: HistoryQuery,
        result: oneshot::Sender<Result<Response, Error>>,
    },
    MarkRead {
        target: String,
        time: SystemTime,
        result: oneshot::Sender<Result<Response, Error>>,
    },
    ReadMarker {
        target: String,
        result: ReadMarkerSender,
    },
    SendText {
        kind: TextKind,
        target: String,
        text: String,
        tags: Vec<(String, String)>,
        result: oneshot::Sender<Result<Sent, Error>>,
    },
    Typing {
        target: String,
        state: Typing,
        result: oneshot::Sender<Result<(), Error>>,
    },
    WaitChannelSync {
        channel: String,
        result: oneshot::Sender<Result<(), Error>>,
    },
    Quit {
        reason: Option<String>,
        result: oneshot::Sender<Result<(), Error>>,
    },
}

enum PendingRequest {
    Response(oneshot::Sender<Result<Response, Error>>),
    Text(oneshot::Sender<Result<Sent, Error>>),
}

impl PendingRequest {
    fn respond(self, response: Response) {
        match self {
            Self::Response(result) => _ = result.send(Ok(response)),
            Self::Text(result) => _ = result.send(Ok(Sent::from_response(&response))),
        }
    }

    fn fail(self, error: Error) {
        match self {
            Self::Response(result) => _ = result.send(Err(error)),
            Self::Text(result) => _ = result.send(Err(error)),
        }
    }
}

struct Quit {
    deadline: Instant,
    results: Vec<oneshot::Sender<Result<(), Error>>>,
}

struct EventSink {
    regular: mpsc::Sender<Event>,
    disconnected: oneshot::Sender<Event>,
    connected: Arc<AtomicBool>,
    state: Arc<RwLock<Arc<State>>>,
    commands: mpsc::WeakSender<Command>,
}

struct Channels {
    client: Client,
    events: Events,
    commands: mpsc::Receiver<Command>,
    event_sink: EventSink,
}

pub(crate) type RegistrationSender = oneshot::Sender<Result<(), Error>>;

#[derive(Default)]
struct RegistrationWait(Option<RegistrationSender>);

impl RegistrationWait {
    fn succeeded(&mut self) {
        if let Some(sender) = self.0.take() {
            _ = sender.send(Ok(()));
        }
    }

    fn failed(&mut self, error: Error) {
        if let Some(sender) = self.0.take() {
            _ = sender.send(Err(error));
        }
    }
}

#[derive(Clone)]
pub(crate) struct ConnectionPlan {
    pub(crate) address: ServerAddress,
    pub(crate) tls: TlsConfig,
}

#[derive(Default)]
pub(crate) struct PostRegistration {
    pub(crate) bot: bool,
    pub(crate) channels: Vec<String>,
    pub(crate) history_on_reconnect: bool,
}

/// A cheap, cloneable handle to a running IRC client.
///
/// Typed requests use server labels when available and otherwise match protocol
/// reply boundaries. Concurrent identical unlabelled requests resolve in FIFO order.
#[derive(Clone)]
pub struct Client {
    commands: mpsc::Sender<Command>,
    connected: Arc<AtomicBool>,
    state: Arc<RwLock<Arc<State>>>,
}

/// Server-assigned metadata for a sent chat message.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Sent {
    /// Network-unique identifier from a labelled echo.
    pub msgid: Option<String>,
    /// Server time from a labelled echo.
    pub time: Option<SystemTime>,
}

/// Outcome of an account registration request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountRegistration {
    /// The account was created and authenticated.
    Registered {
        /// Registered account name.
        account: String,
    },
    /// The server requires an out-of-band verification code.
    VerificationRequired {
        /// Account awaiting verification.
        account: String,
        /// Human-readable verification instructions.
        message: String,
    },
}

/// Handle for account persistence requests.
pub struct Persistence<'a> {
    client: &'a Client,
}

impl Persistence<'_> {
    /// Requests the server's current persistence status.
    ///
    /// # Errors
    ///
    /// Returns an error when the request is unsupported or interrupted.
    pub async fn get(&self) -> Result<papo_core::PersistenceStatus, Error> {
        self.client
            .core_request(CoreRequest::PersistenceGet)
            .await?;
        self.state().ok_or(Error::Disconnected)
    }

    /// Changes the account persistence preference.
    ///
    /// # Errors
    ///
    /// Returns an error when the request is unsupported or interrupted.
    pub async fn set(
        &self,
        setting: papo_core::PersistenceSetting,
    ) -> Result<papo_core::PersistenceStatus, Error> {
        self.client
            .core_request(CoreRequest::PersistenceSet(setting))
            .await?;
        self.state().ok_or(Error::Disconnected)
    }

    /// Returns the most recently reported persistence status.
    #[must_use]
    pub fn state(&self) -> Option<papo_core::PersistenceStatus> {
        self.client.state(crate::State::persistence)
    }
}

impl Sent {
    fn from_response(response: &Response) -> Self {
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

    /// Returns a handle for account persistence requests and cached state.
    #[must_use]
    pub const fn persistence(&self) -> Persistence<'_> {
        Persistence { client: self }
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

    /// Creates an account through `draft/account-registration`.
    ///
    /// # Errors
    ///
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn register(
        &self,
        account: Option<&str>,
        email: Option<&str>,
        password: papo_core::Secret,
    ) -> Result<AccountRegistration, Error> {
        let response = self
            .core_request(CoreRequest::Register {
                account: account.map(str::to_owned),
                email: email.map(str::to_owned),
                password,
            })
            .await?;
        account_registration_result(&response)
    }

    /// Submits an account verification code.
    ///
    /// # Errors
    ///
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn verify(&self, account: &str, code: &str) -> Result<AccountRegistration, Error> {
        let response = self
            .core_request(CoreRequest::Verify {
                account: account.to_owned(),
                code: code.to_owned(),
            })
            .await?;
        account_registration_result(&response)
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
    /// field of [`Event::Disconnected`].
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

fn account_registration_result(response: &Response) -> Result<AccountRegistration, Error> {
    if let Some(error) = response.account_registration_error() {
        return Err(Error::AccountRegistration(error));
    }
    let message = response.lines.first().ok_or(Error::Disconnected)?.as_ref();
    let account = message.param(1).ok_or(Error::Disconnected)?.to_owned();
    match message.param(0) {
        Some("SUCCESS") => Ok(AccountRegistration::Registered { account }),
        Some("VERIFICATION_REQUIRED") => Ok(AccountRegistration::VerificationRequired {
            account,
            message: message.param(2).unwrap_or_default().to_owned(),
        }),
        _ => response
            .error()
            .map_or(Err(Error::Disconnected), |error| Err(Error::Server(error))),
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Client").finish_non_exhaustive()
    }
}

/// The events produced by a running IRC client.
///
/// Delivery is bounded. If events are not consumed, socket reads stall while
/// client commands and session timers continue to be served.
pub struct Events {
    receiver: mpsc::Receiver<Event>,
    disconnected: Option<oneshot::Receiver<Event>>,
}

impl Events {
    /// Waits for the next event, or returns `None` after the driver stops.
    pub async fn next(&mut self) -> Option<Event> {
        if let Some(event) = self.receiver.recv().await {
            return Some(event);
        }
        self.disconnected.take()?.await.ok()
    }
}

/// A future that owns and drives an IRC session and its transport.
#[must_use = "a Driver does nothing unless awaited or spawned"]
pub struct Driver {
    inner: Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>,
}

impl Driver {
    /// Creates handles and a driver for an already-connected stream.
    pub fn new(stream: transport::Stream, session: Session) -> (Client, Events, Self) {
        Self::with_stream(stream, session)
    }

    fn with_stream<S>(stream: S, session: Session) -> (Client, Events, Self)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        Self::with_stream_and_capacity(stream, session, DEFAULT_CHANNEL_CAPACITY)
    }

    fn with_stream_and_capacity<S>(
        stream: S,
        session: Session,
        capacity: usize,
    ) -> (Client, Events, Self)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        Self::with_stream_and_registration(
            stream,
            session,
            capacity,
            None,
            PostRegistration::default(),
        )
    }

    fn with_stream_and_registration<S>(
        stream: S,
        session: Session,
        capacity: usize,
        registration: Option<RegistrationSender>,
        defaults: PostRegistration,
    ) -> (Client, Events, Self)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let channels = channels(capacity, &session);
        let inner = Box::pin(run(
            stream,
            session,
            channels.commands,
            channels.event_sink,
            RegistrationWait(registration),
            defaults,
        ));
        (channels.client, channels.events, Self { inner })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_plan(
        plan: Result<ConnectionPlan, AddressError>,
        session: Session,
        capacity: usize,
        registration: Option<RegistrationSender>,
        defaults: PostRegistration,
        reconnect: Reconnect,
        disconnected_queue_capacity: usize,
        sts_store: Box<dyn StsStore + Send>,
        force_plaintext: bool,
    ) -> (Client, Events, Self) {
        Self::with_connector(
            plan,
            session,
            capacity,
            registration,
            defaults,
            reconnect,
            disconnected_queue_capacity,
            sts_store,
            force_plaintext,
            Arc::new(|plan| {
                Box::pin(async move {
                    open_connection(plan)
                        .await
                        .map(|stream| Box::new(stream) as BoxConnection)
                })
            }),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_connector(
        plan: Result<ConnectionPlan, AddressError>,
        session: Session,
        capacity: usize,
        registration: Option<RegistrationSender>,
        defaults: PostRegistration,
        reconnect: Reconnect,
        disconnected_queue_capacity: usize,
        sts_store: Box<dyn StsStore + Send>,
        force_plaintext: bool,
        connector: Connector,
    ) -> (Client, Events, Self) {
        Self::with_connector_and_entropy(
            plan,
            session,
            capacity,
            registration,
            defaults,
            reconnect,
            disconnected_queue_capacity,
            sts_store,
            force_plaintext,
            connector,
            Box::new(crate::OsEntropy::next_u64),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_connector_and_entropy(
        plan: Result<ConnectionPlan, AddressError>,
        session: Session,
        capacity: usize,
        registration: Option<RegistrationSender>,
        defaults: PostRegistration,
        reconnect: Reconnect,
        disconnected_queue_capacity: usize,
        sts_store: Box<dyn StsStore + Send>,
        force_plaintext: bool,
        connector: Connector,
        entropy: BackoffEntropy,
    ) -> (Client, Events, Self) {
        let channels = channels(capacity, &session);
        let inner = Box::pin(run_plan(
            plan,
            session,
            channels.commands,
            channels.event_sink,
            RegistrationWait(registration),
            defaults,
            reconnect,
            disconnected_queue_capacity,
            sts_store,
            force_plaintext,
            connector,
            entropy,
        ));
        (channels.client, channels.events, Self { inner })
    }
}

fn channels(capacity: usize, session: &Session) -> Channels {
    let (command_sender, command_receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
    let (event_sender, event_receiver) = mpsc::channel(capacity);
    let (disconnect_sender, disconnect_receiver) = oneshot::channel();
    let connected = Arc::new(AtomicBool::new(false));
    let state = Arc::new(RwLock::new(Arc::new(State::from_session(session, false))));
    let client = Client {
        commands: command_sender,
        connected: Arc::clone(&connected),
        state: Arc::clone(&state),
    };
    let events = EventSink {
        regular: event_sender,
        disconnected: disconnect_sender,
        connected: Arc::clone(&connected),
        state,
        commands: client.commands.downgrade(),
    };
    Channels {
        client,
        events: Events {
            receiver: event_receiver,
            disconnected: Some(disconnect_receiver),
        },
        commands: command_receiver,
        event_sink: events,
    }
}

async fn open_connection(plan: ConnectionPlan) -> Result<transport::Stream, Error> {
    #[cfg(any(
        not(target_family = "wasm"),
        all(target_os = "wasi", not(target_env = "p1"))
    ))]
    {
        if plan.address.websocket_url().is_some() {
            #[cfg(feature = "websocket")]
            {
                let url = plan
                    .address
                    .websocket_url()
                    .expect("WebSocket URL is present");
                return transport::connect_websocket(
                    &url,
                    plan.address.is_secure().then_some(&plan.tls),
                )
                .await
                .map_err(Error::Connect);
            }
            #[cfg(not(feature = "websocket"))]
            return Err(Error::Unsupported("websocket"));
        }
        let security = match plan.address.protocol {
            Protocol::Irc => transport::Security::Plaintext,
            Protocol::Ircs => transport::Security::Tls(plan.tls),
            Protocol::Ws | Protocol::Wss => unreachable!("WebSocket URL handled above"),
        };
        transport::connect(&plan.address.host, plan.address.port, &security)
            .await
            .map_err(Error::Connect)
    }
    #[cfg(not(any(
        not(target_family = "wasm"),
        all(target_os = "wasi", not(target_env = "p1"))
    )))]
    {
        _ = plan;
        Err(Error::Unsupported("outbound sockets"))
    }
}

impl Future for Driver {
    type Output = Result<(), Error>;

    fn poll(mut self: Pin<&mut Self>, context: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        self.inner.as_mut().poll(context)
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_plan(
    plan: Result<ConnectionPlan, AddressError>,
    mut session: Session,
    mut commands: mpsc::Receiver<Command>,
    events: EventSink,
    mut registration: RegistrationWait,
    defaults: PostRegistration,
    reconnect: Reconnect,
    disconnected_queue_capacity: usize,
    mut sts_store: Box<dyn StsStore + Send>,
    force_plaintext: bool,
    connector: Connector,
    mut entropy: BackoffEntropy,
) -> Result<(), Error> {
    let Some(mut base_plan) = initial_connection_plan(plan, &mut registration)? else {
        return Ok(());
    };
    let mut attempts = 0_usize;
    let mut registered = false;
    let mut queued = VecDeque::new();
    let mut sts_required = false;
    let mut last_sts_upgrade = None;
    loop {
        attempts = attempts.saturating_add(1);
        let plan = match plan_with_stored_sts(&base_plan, sts_store.as_mut(), force_plaintext) {
            Ok(plan) => plan,
            Err(error) => return finish_connection_failure(events, &mut registration, error),
        };
        sts_required |= !base_plan.address.is_secure() && plan.address.is_secure();
        let stream = match connect_or_wait(
            connector(plan.clone()),
            &mut commands,
            &mut queued,
            disconnected_queue_capacity,
            &events.regular,
            sts_required,
            reconnect,
            attempts,
            entropy.as_mut(),
        )
        .await
        {
            ConnectionAttempt::Connected(stream) => stream,
            ConnectionAttempt::Retry => continue,
            ConnectionAttempt::Stop => return Ok(()),
            ConnectionAttempt::Failed(error) => {
                return finish_connection_failure(events, &mut registration, error);
            }
        };
        let mut end = run_planned_connection(
            stream,
            &mut session,
            &mut commands,
            &events,
            &mut registration,
            &defaults,
            &mut queued,
            &mut registered,
            &plan,
            sts_store,
        )
        .await;
        sts_store = end
            .sts_store
            .take()
            .expect("planned connections retain their STS store");
        reschedule_stored_policy(sts_store.as_mut(), &plan.address.host);
        let (end, upgrade_port) = match triage_connection_end(
            end,
            &plan,
            force_plaintext,
            reconnect,
            &mut attempts,
            &mut last_sts_upgrade,
        ) {
            ControlFlow::Break((end, registration_failed)) => {
                if registration_failed {
                    registration.failed(end.registration_error());
                }
                return finish(events, end, false);
            }
            ControlFlow::Continue(retry) => retry,
        };
        if let Some(port) = upgrade_port {
            base_plan.address.set_tls(true);
            base_plan.address.set_port(port);
            sts_required = true;
        }
        _ = events
            .regular
            .send(Event::Disconnected {
                reason: end.reason,
                will_retry: true,
            })
            .await;
        if upgrade_port.is_some() {
            continue;
        }
        if !wait_before_retry(
            reconnect,
            attempts,
            &mut commands,
            &mut queued,
            disconnected_queue_capacity,
            entropy.as_mut(),
        )
        .await
        {
            return Ok(());
        }
    }
}

fn initial_connection_plan(
    plan: Result<ConnectionPlan, AddressError>,
    registration: &mut RegistrationWait,
) -> Result<Option<ConnectionPlan>, Error> {
    match plan {
        Ok(plan) => Ok(Some(plan)),
        Err(error) if registration.0.is_some() => {
            registration.failed(Error::Address(error));
            Ok(None)
        }
        Err(error) => Err(Error::Address(error)),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_planned_connection(
    stream: BoxConnection,
    session: &mut Session,
    commands: &mut mpsc::Receiver<Command>,
    events: &EventSink,
    registration: &mut RegistrationWait,
    defaults: &PostRegistration,
    queued: &mut VecDeque<Message>,
    registered: &mut bool,
    plan: &ConnectionPlan,
    sts_store: Box<dyn StsStore + Send>,
) -> ConnectionEnd {
    let security = if plan.address.is_secure() {
        ConnectionSecurity::Secure
    } else {
        ConnectionSecurity::Plaintext
    };
    run_connection(
        stream,
        session,
        commands,
        events,
        registration,
        defaults,
        queued,
        registered,
        Some(Connection {
            host: plan.address.host.clone(),
            security,
        }),
        Some(sts_store),
        plan.address.port,
        &events.connected,
    )
    .await
}

fn triage_connection_end(
    mut end: ConnectionEnd,
    plan: &ConnectionPlan,
    force_plaintext: bool,
    reconnect: Reconnect,
    attempts: &mut usize,
    last_sts_upgrade: &mut Option<String>,
) -> ControlFlow<(ConnectionEnd, bool), (ConnectionEnd, Option<u16>)> {
    if end.quit.is_some() {
        return ControlFlow::Break((end, false));
    }
    if end.registered_at.is_some() {
        *last_sts_upgrade = None;
    }
    if let Some(port) = end.sts.upgrade {
        if force_plaintext {
            let error = Error::StsRequiresTls {
                host: plan.address.host.clone(),
            };
            end.reason = error.to_string();
            end.error = error;
        } else if let Err(error) = record_sts_upgrade(last_sts_upgrade, &plan.address.host) {
            end.reason = error.to_string();
            end.error = error;
        } else if reconnect.permits(*attempts) {
            return ControlFlow::Continue((end, Some(port)));
        }
        return ControlFlow::Break((end, true));
    }
    *last_sts_upgrade = None;
    if is_fatal_failure(&end.error, end.close_kind) {
        return ControlFlow::Break((end, true));
    }
    if end
        .registered_at
        .is_some_and(|at| now().saturating_duration_since(at) >= STABLE_CONNECTION)
    {
        *attempts = 0;
    }
    if !reconnect.permits(*attempts) {
        return ControlFlow::Break((end, true));
    }
    ControlFlow::Continue((end, None))
}

enum ConnectionAttempt {
    Connected(BoxConnection),
    Retry,
    Stop,
    Failed(Error),
}

#[allow(clippy::too_many_arguments)]
async fn connect_or_wait(
    connection: ConnectFuture,
    commands: &mut mpsc::Receiver<Command>,
    queued: &mut VecDeque<Message>,
    capacity: usize,
    events: &mpsc::Sender<Event>,
    sts_required: bool,
    reconnect: Reconnect,
    attempts: usize,
    entropy: &mut (dyn FnMut() -> u64 + Send),
) -> ConnectionAttempt {
    let Some(result) = connect_while_disconnected(connection, commands, queued, capacity).await
    else {
        return ConnectionAttempt::Stop;
    };
    let error = match result {
        Ok(stream) => return ConnectionAttempt::Connected(stream),
        Err(error) => error,
    };
    let reason = error.to_string();
    if sts_required || is_fatal_failure(&error, None) || !reconnect.permits(attempts) {
        return ConnectionAttempt::Failed(error);
    }
    _ = events
        .send(Event::Disconnected {
            reason,
            will_retry: true,
        })
        .await;
    if wait_before_retry(reconnect, attempts, commands, queued, capacity, entropy).await {
        ConnectionAttempt::Retry
    } else {
        ConnectionAttempt::Stop
    }
}

fn record_sts_upgrade(last_host: &mut Option<String>, host: &str) -> Result<(), Error> {
    if last_host.as_deref() == Some(host) {
        return Err(Error::StsUpgradeLoop {
            host: host.to_owned(),
        });
    }
    *last_host = Some(host.to_owned());
    Ok(())
}

fn finish_connection_failure(
    events: EventSink,
    registration: &mut RegistrationWait,
    error: Error,
) -> Result<(), Error> {
    let reason = error.to_string();
    if registration.0.is_some() {
        registration.failed(error);
        drop(events.regular);
        _ = events.disconnected.send(Event::Disconnected {
            reason,
            will_retry: false,
        });
        return Ok(());
    }
    finish(
        events,
        ConnectionEnd {
            reason,
            error,
            close_kind: None,
            quit: None,
            quit_timed_out: false,
            registered_at: None,
            sts: StsActions::default(),
            sts_store: None,
        },
        false,
    )
}

fn apply_sts_action(store: &mut dyn StsStore, port: u16, action: &StsAction) {
    match action {
        StsAction::Persist { host, duration } => {
            let now = SystemTime::now();
            if let Some(expires_at) = now.checked_add(*duration) {
                store.put(StoredPolicy {
                    host: host.clone(),
                    port,
                    expires_at,
                    duration: *duration,
                });
            }
        }
        StsAction::Remove { host } => store.remove(host),
        _ => {}
    }
}

fn reschedule_stored_policy(store: &mut dyn StsStore, host: &str) {
    if let Some(policy) = store.get(host) {
        store.put(reschedule_on_disconnect(policy, SystemTime::now()));
    }
}

fn plan_with_stored_sts(
    base: &ConnectionPlan,
    store: &mut dyn StsStore,
    force_plaintext: bool,
) -> Result<ConnectionPlan, Error> {
    let Some(policy) = store.get(&base.address.host) else {
        return Ok(base.clone());
    };
    if !policy.is_active_at(SystemTime::now()) {
        store.remove(&base.address.host);
        return Ok(base.clone());
    }
    if base.address.is_secure() {
        return Ok(base.clone());
    }
    if force_plaintext {
        return Err(Error::StsRequiresTls {
            host: base.address.host.clone(),
        });
    }
    let mut plan = base.clone();
    plan.address.set_tls(true);
    plan.address.set_port(policy.port);
    Ok(plan)
}

fn is_fatal_failure(error: &Error, close_kind: Option<papo_core::CloseKind>) -> bool {
    if matches!(
        close_kind,
        Some(papo_core::CloseKind::SaslFailed | papo_core::CloseKind::AccountRequired)
    ) {
        return true;
    }
    #[cfg(tls)]
    if matches!(
        error,
        Error::Connect(transport::ConnectError::Tls(
            rustls::Error::InvalidCertificate(_)
        ))
    ) {
        return true;
    }
    false
}

async fn connect_while_disconnected(
    mut connection: ConnectFuture,
    commands: &mut mpsc::Receiver<Command>,
    queued: &mut VecDeque<Message>,
    capacity: usize,
) -> Option<Result<BoxConnection, Error>> {
    loop {
        tokio::select! {
            result = &mut connection => return Some(result),
            command = commands.recv() => {
                if !handle_disconnected_command(command, queued, capacity) {
                    return None;
                }
            }
        }
    }
}

async fn wait_before_retry(
    reconnect: Reconnect,
    attempts: usize,
    commands: &mut mpsc::Receiver<Command>,
    queued: &mut VecDeque<Message>,
    capacity: usize,
    entropy: &mut (dyn FnMut() -> u64 + Send),
) -> bool {
    let retry = u32::try_from(attempts.saturating_sub(1)).unwrap_or(u32::MAX);
    let sleep = tokio::time::sleep(reconnect.delay(retry, entropy()));
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            () = &mut sleep => return true,
            command = commands.recv() => {
                if !handle_disconnected_command(command, queued, capacity) {
                    return false;
                }
            }
        }
    }
}

fn handle_disconnected_command(
    command: Option<Command>,
    queued: &mut VecDeque<Message>,
    capacity: usize,
) -> bool {
    match command {
        Some(Command::Send { message, result }) => {
            let response = message.to_line().map_err(Error::Build).and_then(|_| {
                if queued.len() < capacity {
                    queued.push_back(message);
                    Ok(())
                } else {
                    Err(Error::Disconnected)
                }
            });
            _ = result.send(response);
            true
        }
        Some(
            Command::Request { result, .. }
            | Command::CoreRequest { result, .. }
            | Command::History { result, .. }
            | Command::MarkRead { result, .. },
        ) => {
            _ = result.send(Err(Error::Disconnected));
            true
        }
        Some(Command::SendText { result, .. }) => {
            _ = result.send(Err(Error::Disconnected));
            true
        }
        Some(Command::ReadMarker { result, .. }) => {
            _ = result.send(Err(Error::Disconnected));
            true
        }
        Some(Command::Typing { result, .. } | Command::WaitChannelSync { result, .. }) => {
            _ = result.send(Err(Error::Disconnected));
            true
        }
        Some(Command::Quit { result, .. }) => {
            _ = result.send(Ok(()));
            false
        }
        None => false,
    }
}

async fn run<S>(
    stream: S,
    mut session: Session,
    mut commands: mpsc::Receiver<Command>,
    events: EventSink,
    mut registration: RegistrationWait,
    defaults: PostRegistration,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut registered = false;
    let mut queued = VecDeque::new();
    let end = run_connection(
        stream,
        &mut session,
        &mut commands,
        &events,
        &mut registration,
        &defaults,
        &mut queued,
        &mut registered,
        None,
        None,
        0,
        &events.connected,
    )
    .await;
    registration.failed(end.registration_error());
    finish(events, end, false)
}

struct ConnectionEnd {
    reason: String,
    error: Error,
    close_kind: Option<papo_core::CloseKind>,
    quit: Option<Quit>,
    quit_timed_out: bool,
    #[allow(dead_code)]
    registered_at: Option<Instant>,
    sts: StsActions,
    sts_store: Option<Box<dyn StsStore + Send>>,
}

#[derive(Default)]
struct StsActions {
    upgrade: Option<u16>,
}

impl ConnectionEnd {
    fn registration_error(&self) -> Error {
        Error::Registration(self.reason.clone())
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn run_connection<S>(
    mut stream: S,
    session: &mut Session,
    commands: &mut mpsc::Receiver<Command>,
    events: &EventSink,
    registration: &mut RegistrationWait,
    defaults: &PostRegistration,
    queued: &mut VecDeque<Message>,
    registered: &mut bool,
    connection: Option<Connection>,
    mut sts_store: Option<Box<dyn StsStore + Send>>,
    sts_port: u16,
    connected: &AtomicBool,
) -> ConnectionEnd
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let current = now();
    match connection {
        Some(connection) => session.handle_connected_with(current, connection),
        None => session.handle_connected(current),
    }
    connected.store(true, Ordering::Relaxed);
    publish_state(events, session, true);
    while let Some(message) = queued.pop_front() {
        session
            .send(now(), message)
            .expect("disconnected commands were validated before queueing");
    }
    let mut commands_open = true;
    let mut quit = None;
    let mut pending_event = None;
    let mut requests = BTreeMap::new();
    let mut active_typing = BTreeMap::new();
    let mut sync_waiters = Vec::new();
    let mut read_marker_waiters = Vec::new();
    let mut catchups = BTreeMap::new();
    let mut registered_at = None;
    let mut sts = StsActions::default();
    let mut buffer = [0; 4096];
    let (mut reason, mut error, close_kind, quit_timed_out) = loop {
        match drain(
            &mut stream,
            session,
            events,
            &mut pending_event,
            &mut requests,
            &mut sync_waiters,
            &mut read_marker_waiters,
            &mut catchups,
            registration,
            defaults,
            registered,
            &mut registered_at,
            &mut sts,
            &mut sts_store,
            sts_port,
        )
        .await
        {
            Ok(Some((kind, reason))) => break (reason, Error::Disconnected, Some(kind), false),
            Ok(None) => {}
            Err(error) => break (error.to_string(), Error::Io(error), None, false),
        }
        let deadline = [
            session.poll_timeout(),
            quit.as_ref().map(|quit: &Quit| quit.deadline),
        ]
        .into_iter()
        .flatten()
        .min();
        tokio::select! {
            read = stream.read(&mut buffer), if pending_event.is_none() => match read {
                Ok(0) => {
                    break (
                        "server closed the connection".to_owned(),
                        Error::Disconnected,
                        None,
                        false,
                    );
                }
                Ok(count) => session.handle_bytes(now(), &buffer[..count]),
                Err(error) => break (error.to_string(), Error::Io(error), None, false),
            },
            permit = events.regular.clone().reserve_owned(), if pending_event.is_some() => match permit {
                Ok(permit) => {
                    _ = permit.send(pending_event.take().expect("pending event is present"));
                }
                Err(_) => pending_event = None,
            },
            command = commands.recv(), if commands_open => match command {
                Some(Command::Send { message, result }) => {
                    _ = result.send(session.send(now(), message).map_err(Error::Build));
                }
                Some(Command::Request { message, matcher, result }) => {
                    let requested = match matcher {
                        Some(matcher) => session.request_with_matcher(now(), message, matcher),
                        None if session.caps().contains_key("labeled-response") => {
                            session.request(now(), message)
                        }
                        None => {
                            _ = result.send(Err(Error::Unsupported("labeled-response")));
                            continue;
                        }
                    };
                    match requested {
                        Ok(id) => {
                            requests.insert(id, PendingRequest::Response(result));
                        }
                        Err(error) => {
                            let error = match error {
                                RequestError::Build(error) => Error::Build(error),
                                _ => Error::RequestLimit,
                            };
                            _ = result.send(Err(error));
                        }
                    }
                }
                Some(Command::CoreRequest { request, result }) => {
                    match request.start(session) {
                        Ok(id) => { requests.insert(id, PendingRequest::Response(result)); }
                        Err(RequestError::Unsupported(feature)) => { _ = result.send(Err(Error::Unsupported(feature))); }
                        Err(RequestError::Build(error)) => { _ = result.send(Err(Error::Build(error))); }
                        Err(_) => { _ = result.send(Err(Error::RequestLimit)); }
                    }
                }
                Some(Command::History { query, result }) => {
                    match session.chathistory(now(), query) {
                        Ok(id) => { requests.insert(id, PendingRequest::Response(result)); }
                        Err(RequestError::Unsupported(feature)) => { _ = result.send(Err(Error::Unsupported(feature))); }
                        Err(RequestError::Build(error)) => { _ = result.send(Err(Error::Build(error))); }
                        Err(RequestError::InvalidMessageReference) => { _ = result.send(Err(Error::Unsupported("message reference"))); }
                        Err(_) => { _ = result.send(Err(Error::RequestLimit)); }
                    }
                }
                Some(Command::MarkRead { target, time, result }) => {
                    match session.mark_read(now(), &target, time) {
                        Ok(id) => { requests.insert(id, PendingRequest::Response(result)); }
                        Err(RequestError::Unsupported(feature)) => { _ = result.send(Err(Error::Unsupported(feature))); }
                        Err(RequestError::Build(error)) => { _ = result.send(Err(Error::Build(error))); }
                        Err(_) => { _ = result.send(Err(Error::RequestLimit)); }
                    }
                }
                Some(Command::ReadMarker { target, result }) => {
                    match session.read_marker(&target) {
                        Ok(()) => match session.state().read_marker(&target) {
                            Some(marker) => { _ = result.send(Ok(marker.time)); }
                            None if read_marker_waiters.len() < 256 => read_marker_waiters.push((target, result)),
                            None => { _ = result.send(Err(Error::RequestLimit)); }
                        },
                        Err(RequestError::Unsupported(feature)) => { _ = result.send(Err(Error::Unsupported(feature))); }
                        Err(RequestError::Build(error)) => { _ = result.send(Err(Error::Build(error))); }
                        Err(_) => { _ = result.send(Err(Error::RequestLimit)); }
                    }
                }
                Some(Command::SendText { kind, target, text, tags, result }) => {
                    let tags: Vec<_> = tags.iter().map(|(key, value)| (key.as_str(), value.as_str())).collect();
                    match session.send_text(now(), kind, &target, &text, &tags) {
                        Ok(Some(id)) => {
                            requests.insert(id, PendingRequest::Text(result));
                        }
                        Ok(None) => _ = result.send(Ok(Sent::default())),
                        Err(error) => {
                            let error = match error {
                                RequestError::Build(error) => Error::Build(error),
                                RequestError::EmptyText => Error::EmptyText,
                                _ => Error::RequestLimit,
                            };
                            _ = result.send(Err(error));
                        }
                    }
                }
                Some(Command::Typing { target, state, result }) => {
                    let current = now();
                    let key = session.isupport().casemapping().fold(&target).into_owned();
                    let send = state != Typing::Active
                        || active_typing.get(&key).is_none_or(|last: &Instant| {
                            last.checked_add(Duration::from_secs(3))
                                .is_none_or(|deadline| current >= deadline)
                        });
                    if send && state == Typing::Active {
                        active_typing.insert(key, current);
                    }
                    let response = if send {
                        let value = match state {
                            Typing::Active => "active",
                            Typing::Paused => "paused",
                            Typing::Done => "done",
                        };
                        session
                            .send(current, Message::tagmsg(target).with_tag("+typing", value))
                            .map_err(Error::Build)
                    } else {
                        Ok(())
                    };
                    _ = result.send(response);
                }
                Some(Command::WaitChannelSync { channel, result }) => {
                    if session
                        .state()
                        .channel(&channel)
                        .is_some_and(papo_core::Channel::is_synced)
                    {
                        _ = result.send(Ok(()));
                    } else {
                        sync_waiters.push((channel, result));
                    }
                }
                Some(Command::Quit { reason, result }) => {
                    request_quit(session, &mut quit, reason, Some(result));
                }
                None => {
                    commands_open = false;
                    request_quit(session, &mut quit, None, None);
                }
            },
            () = wait_until(deadline) => {
                let current = now();
                if quit.as_ref().is_some_and(|quit| current >= quit.deadline) {
                    break (
                        "QUIT was not acknowledged within 10 seconds".to_owned(),
                        Error::Timeout,
                        None,
                        true,
                    );
                }
                session.handle_timeout(current);
            },
        }
    };
    session.handle_disconnected(now());
    for (_, result) in requests {
        result.fail(Error::Disconnected);
    }
    for (_, result) in sync_waiters {
        _ = result.send(Err(Error::Disconnected));
    }
    for (_, result) in read_marker_waiters {
        _ = result.send(Err(Error::Disconnected));
    }
    connected.store(false, Ordering::Relaxed);
    publish_state(events, session, false);
    let close_error = stream.flush().await.err().or(stream.shutdown().await.err());
    if let Some(close_error) = close_error
        && !matches!(error, Error::Io(_))
    {
        reason = close_error.to_string();
        error = Error::Io(close_error);
    }
    ConnectionEnd {
        reason,
        error,
        close_kind,
        quit,
        quit_timed_out,
        registered_at,
        sts,
        sts_store,
    }
}

fn request_quit(
    session: &mut Session,
    quit: &mut Option<Quit>,
    reason: Option<String>,
    result: Option<oneshot::Sender<Result<(), Error>>>,
) {
    if let Some(quit) = quit {
        if let Some(result) = result {
            quit.results.push(result);
        }
        return;
    }
    let message = reason.map_or_else(
        || Message::new("QUIT", Vec::<String>::new()),
        |reason| Message::new("QUIT", [reason]),
    );
    let current = now();
    match session.send(current, message) {
        Ok(()) => {
            *quit = Some(Quit {
                deadline: current + QUIT_TIMEOUT,
                results: result.into_iter().collect(),
            });
        }
        Err(error) => {
            if let Some(result) = result {
                _ = result.send(Err(Error::Build(error)));
            }
        }
    }
}

async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await,
        None => std::future::pending().await,
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn drain<S>(
    stream: &mut S,
    session: &mut Session,
    events: &EventSink,
    pending_event: &mut Option<Event>,
    requests: &mut BTreeMap<RequestId, PendingRequest>,
    sync_waiters: &mut Vec<(String, oneshot::Sender<Result<(), Error>>)>,
    read_marker_waiters: &mut ReadMarkerWaiters,
    catchups: &mut BTreeMap<RequestId, Catchup>,
    registration: &mut RegistrationWait,
    defaults: &PostRegistration,
    registered: &mut bool,
    registered_at: &mut Option<Instant>,
    sts: &mut StsActions,
    sts_store: &mut Option<Box<dyn StsStore + Send>>,
    sts_port: u16,
) -> io::Result<Option<(papo_core::CloseKind, String)>>
where
    S: AsyncWrite + Unpin,
{
    publish_state(events, session, true);
    while let Some(line) = session.poll_transmit() {
        stream.write_all(&line).await?;
    }
    while pending_event.is_none()
        && let Some(core_event) = session.poll_event()
    {
        match event::from_core(core_event) {
            event::CoreEvent::Forward(mut event) => {
                event = match event {
                    Event::Response(response) => {
                        if let Some(catchup) = catchups.remove(&response.id) {
                            match advance_catchup(session, events, catchups, catchup, response) {
                                Some(event) => event,
                                None => continue,
                            }
                        } else if let Some(result) = requests.remove(&response.id) {
                            result.respond(response);
                            continue;
                        } else {
                            Event::Response(response)
                        }
                    }
                    Event::RequestTimedOut { id } => {
                        if let Some(result) = requests.remove(&id) {
                            result.fail(Error::Timeout);
                            continue;
                        }
                        Event::RequestTimedOut { id }
                    }
                    Event::RequestAborted { id } => {
                        if let Some(result) = requests.remove(&id) {
                            result.fail(Error::Disconnected);
                            continue;
                        }
                        Event::RequestAborted { id }
                    }
                    event => event,
                };
                if let Event::ChannelSynced { channel } = &event {
                    resolve_sync_waiters(session, sync_waiters, channel);
                }
                if let Event::ReadMarker(marker) = &event {
                    resolve_read_marker_waiters(session, read_marker_waiters, marker);
                }
                if let Event::Raw(message) = event {
                    event = match events.commands.upgrade() {
                        Some(commands) => crate::MessageEvent::new(
                            message,
                            session.isupport(),
                            session.nick(),
                            Client {
                                commands,
                                connected: Arc::clone(&events.connected),
                                state: Arc::clone(&events.state),
                            },
                            session.caps(),
                            SystemTime::now(),
                        )
                        .map_or_else(Event::Raw, Event::Message),
                        None => Event::Raw(message),
                    };
                }
                match &event {
                    Event::StsUpgrade { port } => sts.upgrade = Some(*port),
                    Event::StsPolicy(action) => {
                        if let Some(store) = sts_store {
                            apply_sts_action(store.as_mut(), sts_port, action);
                        }
                    }
                    _ => {}
                }
                if let Event::Registered(welcome) = &event {
                    after_registration(session, welcome, defaults)?;
                    registration.succeeded();
                    *registered_at = Some(now());
                    if *registered {
                        begin_catchup(session, defaults, catchups);
                        event = Event::Reconnected(welcome.clone());
                    } else {
                        *registered = true;
                    }
                }
                *pending_event = Some(event);
            }
            event::CoreEvent::Message(message) => {
                let event = match events.commands.upgrade() {
                    Some(commands) => crate::MessageEvent::from_core(
                        message,
                        session.isupport(),
                        Client {
                            commands,
                            connected: Arc::clone(&events.connected),
                            state: Arc::clone(&events.state),
                        },
                        session.caps(),
                        SystemTime::now(),
                    )
                    .map_or_else(Event::Raw, Event::Message),
                    None => Event::Raw(message.into_message()),
                };
                *pending_event = Some(event);
            }
            event::CoreEvent::History {
                target,
                messages,
                end,
            } => {
                if catchups.values().any(|catchup| {
                    session
                        .isupport()
                        .casemapping()
                        .eq(&catchup.target, &target)
                }) {
                    continue;
                }
                let Some(commands) = events.commands.upgrade() else {
                    continue;
                };
                let client = Client {
                    commands,
                    connected: Arc::clone(&events.connected),
                    state: Arc::clone(&events.state),
                };
                let messages = messages
                    .into_iter()
                    .filter_map(|item| {
                        crate::MessageEvent::from_replay(
                            item.message,
                            client.clone(),
                            SystemTime::now(),
                        )
                        .ok()
                    })
                    .collect();
                *pending_event = Some(Event::History {
                    target,
                    messages,
                    end,
                });
            }
            event::CoreEvent::Close { kind, reason } => return Ok(Some((kind, reason))),
            event::CoreEvent::Unknown => {}
        }
    }
    Ok(None)
}

fn resolve_sync_waiters(
    session: &Session,
    waiters: &mut Vec<(String, oneshot::Sender<Result<(), Error>>)>,
    channel: &str,
) {
    let mapping = session.isupport().casemapping();
    *waiters = std::mem::take(waiters)
        .into_iter()
        .filter_map(|(waiting, result)| {
            if mapping.eq(&waiting, channel) {
                _ = result.send(Ok(()));
                None
            } else {
                Some((waiting, result))
            }
        })
        .collect();
}

fn resolve_read_marker_waiters(
    session: &Session,
    waiters: &mut ReadMarkerWaiters,
    marker: &papo_core::ReadMarker,
) {
    let mapping = session.isupport().casemapping();
    *waiters = std::mem::take(waiters)
        .into_iter()
        .filter_map(|(target, result)| {
            if mapping.eq(&target, &marker.target) {
                _ = result.send(Ok(marker.time));
                None
            } else {
                Some((target, result))
            }
        })
        .collect();
}

fn advance_catchup(
    session: &mut Session,
    events: &EventSink,
    catchups: &mut BTreeMap<RequestId, Catchup>,
    mut catchup: Catchup,
    response: Response,
) -> Option<Event> {
    let page = response.history?;
    let next = extend_catchup(
        &mut catchup,
        page.messages.into_iter().map(|item| item.message),
    );
    if !page.end
        && catchup.messages.len() < 1_000
        && let Some(next) = next
    {
        let query = HistoryQuery::After {
            target: catchup.target.clone(),
            after: papo_core::MsgRef::Msgid(next),
            limit: (1_000 - catchup.messages.len()).min(100),
        };
        if let Ok(id) = session.chathistory(now(), query) {
            catchups.insert(id, catchup);
            return None;
        }
    }
    let commands = events.commands.upgrade()?;
    let client = Client {
        commands,
        connected: Arc::clone(&events.connected),
        state: Arc::clone(&events.state),
    };
    let messages = catchup
        .messages
        .into_iter()
        .filter_map(|message| {
            crate::MessageEvent::from_history(message, client.clone(), SystemTime::now()).ok()
        })
        .collect();
    Some(Event::History {
        target: catchup.target,
        messages,
        end: page.end,
    })
}

fn extend_catchup(
    catchup: &mut Catchup,
    messages: impl IntoIterator<Item = Message>,
) -> Option<String> {
    let mut next = None;
    for message in messages {
        let msgid = message
            .as_ref()
            .tag("msgid")
            .map(std::borrow::Cow::into_owned);
        next.clone_from(&msgid);
        if msgid
            .as_ref()
            .is_none_or(|msgid| catchup.seen.insert(msgid.clone()))
        {
            catchup.messages.push(message);
        }
        if catchup.messages.len() == 1_000 {
            break;
        }
    }
    next
}

fn begin_catchup(
    session: &mut Session,
    defaults: &PostRegistration,
    catchups: &mut BTreeMap<RequestId, Catchup>,
) {
    if !defaults.history_on_reconnect || !should_fetch_history(session) {
        return;
    }
    let seen = defaults
        .channels
        .iter()
        .filter_map(|channel| session.state().last_seen(channel).cloned())
        .collect::<Vec<_>>();
    for seen in seen {
        let query = HistoryQuery::After {
            target: seen.target.clone(),
            after: papo_core::MsgRef::Msgid(seen.msgid.clone()),
            limit: 100,
        };
        if let Ok(id) = session.chathistory(now(), query) {
            catchups.insert(
                id,
                Catchup {
                    target: seen.target,
                    messages: Vec::new(),
                    seen: HashSet::from([seen.msgid]),
                },
            );
        }
    }
}

fn should_fetch_history(session: &Session) -> bool {
    !session.server_rejoins_channels()
}

fn publish_state(events: &EventSink, session: &Session, connected: bool) {
    let current = events
        .state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if current.matches(session, connected) {
        return;
    }
    drop(current);
    let mut slot = events
        .state
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *slot = Arc::new(State::from_session(session, connected));
}

fn after_registration(
    session: &mut Session,
    welcome: &papo_core::Welcome,
    defaults: &PostRegistration,
) -> Result<(), io::Error> {
    if defaults.bot
        && let Some(mode) = welcome.isupport.bot_mode()
    {
        session
            .send(
                now(),
                Message::new("MODE", [welcome.nick.clone(), format!("+{mode}")]),
            )
            .map_err(io::Error::other)?;
    }
    if should_rejoin(session) {
        for channel in &defaults.channels {
            session
                .send(now(), Message::new("JOIN", [channel.clone()]))
                .map_err(io::Error::other)?;
        }
    }
    Ok(())
}

fn should_rejoin(session: &Session) -> bool {
    !session.server_rejoins_channels()
}

fn finish(events: EventSink, end: ConnectionEnd, will_retry: bool) -> Result<(), Error> {
    drop(events.regular);
    _ = events.disconnected.send(Event::Disconnected {
        reason: end.reason,
        will_retry,
    });
    if let Some(quit) = end.quit {
        for response in quit.results {
            _ = response.send(if end.quit_timed_out {
                Err(Error::Timeout)
            } else {
                Ok(())
            });
        }
        Ok(())
    } else {
        Err(end.error)
    }
}

fn now() -> Instant {
    tokio::time::Instant::now().into_std()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        io,
        pin::Pin,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        task::{Context, Poll},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use papo_core::{
        Config, Decoding, Sasl,
        sts::{ConnectionSecurity, StsAction},
    };
    use papo_proto::{Message, Typing};
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};

    use super::{ConnectionPlan, Driver, Error, Event, Session};
    use crate::{MessageEvent, MessageKind, Origin, Reconnect, transport::TlsConfig};

    struct FailingStream {
        inner: DuplexStream,
        read_error: Arc<AtomicBool>,
    }

    #[derive(Clone, Default)]
    struct SharedStsStore(Arc<Mutex<papo_core::sts::MemoryStsStore>>);

    impl papo_core::sts::StsStore for SharedStsStore {
        fn get(&self, host: &str) -> Option<papo_core::sts::StoredPolicy> {
            papo_core::sts::StsStore::get(&*self.0.lock().unwrap(), host)
        }

        fn put(&mut self, policy: papo_core::sts::StoredPolicy) {
            papo_core::sts::StsStore::put(&mut *self.0.lock().unwrap(), policy);
        }

        fn remove(&mut self, host: &str) {
            papo_core::sts::StsStore::remove(&mut *self.0.lock().unwrap(), host);
        }
    }

    impl AsyncRead for FailingStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            if self.read_error.swap(false, Ordering::Relaxed) {
                Poll::Ready(Err(io::Error::other("original read failure")))
            } else {
                Pin::new(&mut self.inner).poll_read(context, buffer)
            }
        }
    }

    impl AsyncWrite for FailingStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.inner).poll_write(context, buffer)
        }

        fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.inner).poll_flush(context)
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Err(io::Error::other("shutdown failed")))
        }
    }

    fn scripted(config: Config) -> (DuplexStream, super::Client, super::Events, Driver) {
        scripted_with_capacity(config, super::DEFAULT_CHANNEL_CAPACITY)
    }

    fn scripted_with_capacity(
        config: Config,
        capacity: usize,
    ) -> (DuplexStream, super::Client, super::Events, Driver) {
        let (client_stream, server) = tokio::io::duplex(4096);
        let (client, events, driver) =
            Driver::with_stream_and_capacity(client_stream, Session::new(config), capacity);
        (server, client, events, driver)
    }

    fn scripted_with_failures(
        read_error: bool,
    ) -> (DuplexStream, super::Client, super::Events, Driver) {
        let (client_stream, server) = tokio::io::duplex(4096);
        let stream = FailingStream {
            inner: client_stream,
            read_error: Arc::new(AtomicBool::new(read_error)),
        };
        let (client, events, driver) = Driver::with_stream(stream, Session::new(config()));
        (server, client, events, driver)
    }

    async fn registration_burst(server: &mut DuplexStream) {
        let expected = b"CAP LS 302\r\nNICK papo-driver\r\nUSER papo 0 * :papo driver test\r\n";
        let mut actual = vec![0; expected.len()];
        server.read_exact(&mut actual).await.unwrap();
        assert_eq!(actual, expected);
    }

    async fn client_line(server: &mut DuplexStream, expected: &[u8]) {
        let mut actual = vec![0; expected.len()];
        server.read_exact(&mut actual).await.unwrap();
        assert_eq!(actual, expected);
    }

    async fn server_line(server: &mut DuplexStream, line: &[u8]) {
        server.write_all(line).await.unwrap();
    }

    fn changed_snapshot(
        client: &super::Client,
        previous: &Arc<crate::State>,
        change: &str,
    ) -> Arc<crate::State> {
        let current = client.snapshot();
        assert!(!Arc::ptr_eq(previous, &current), "stale after {change}");
        current
    }

    async fn complete_registration(server: &mut DuplexStream, events: &mut super::Events) {
        registration_burst(server).await;
        server_line(server, b":ergo.test CAP * LS :\r\n").await;
        client_line(server, b"CAP END\r\n").await;
        server_line(server, b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
    }

    async fn complete_labelled_registration(server: &mut DuplexStream, events: &mut super::Events) {
        registration_burst(server).await;
        server_line(server, b":ergo.test CAP * LS :labeled-response\r\n").await;
        client_line(server, b"CAP REQ :labeled-response\r\n").await;
        server_line(server, b":ergo.test CAP * ACK :labeled-response\r\n").await;
        client_line(server, b"CAP END\r\n").await;
        server_line(
            server,
            b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
    }

    async fn complete_history_registration(server: &mut DuplexStream, events: &mut super::Events) {
        registration_burst(server).await;
        server_line(
            server,
            b":ergo.test CAP * LS :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        client_line(
            server,
            b"CAP REQ :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        server_line(
            server,
            b":ergo.test CAP * ACK :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        client_line(server, b"CAP END\r\n").await;
        server_line(server, b":ergo.test 005 papo-driver CHATHISTORY=100 MSGREFTYPES=msgid,timestamp :supported\r\n:ergo.test 001 papo-driver :Welcome\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
    }

    async fn complete_read_marker_registration(
        server: &mut DuplexStream,
        events: &mut super::Events,
    ) {
        registration_burst(server).await;
        server_line(
            server,
            b":ergo.test CAP * LS :draft/read-marker labeled-response\r\n",
        )
        .await;
        client_line(server, b"CAP REQ :draft/read-marker labeled-response\r\n").await;
        server_line(
            server,
            b":ergo.test CAP * ACK :draft/read-marker labeled-response\r\n",
        )
        .await;
        client_line(server, b"CAP END\r\n").await;
        server_line(server, b":ergo.test 001 papo-driver :Welcome\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
    }

    async fn history_response(server: &mut DuplexStream, label: usize) {
        let reply = format!(
            "@label={label} :ergo.test BATCH +reply{label} labeled-response\r\n@batch=reply{label};draft/chathistory-end :ergo.test BATCH +history{label} chathistory #papo\r\n@batch=history{label};time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY12{label:04} :emily!~e@localhost PRIVMSG #papo :page {label}\r\n@batch=reply{label} :ergo.test BATCH -history{label}\r\n:ergo.test BATCH -reply{label}\r\n"
        );
        server_line(server, reply.as_bytes()).await;
    }

    async fn assert_join_waits_for_sync(server: &mut DuplexStream, client: &super::Client) {
        let join = tokio::spawn({
            let client = client.clone();
            async move { client.join("#papo").await }
        });
        client_line(server, b"@label=2 JOIN #papo\r\n").await;
        server_line(
            server,
            b"@label=2 :papo-driver!papo@localhost JOIN #papo * :papo driver test\r\n",
        )
        .await;
        tokio::task::yield_now().await;
        assert!(!join.is_finished());
        server_line(server, b":ergo.test 332 papo-driver #papo :Ergonomic IRC\r\n:ergo.test 353 papo-driver = #papo :@papo-driver alice\r\n:ergo.test 366 papo-driver #papo :End of /NAMES list\r\n").await;
        let joined = join.await.unwrap().unwrap();
        assert_eq!(
            (joined.name.as_str(), joined.topic.as_deref()),
            ("#papo", Some("Ergonomic IRC"))
        );
        assert_eq!(joined.members_count, 2);
        client_line(server, b"WHO #papo %tcuhnfar,152\r\n").await;
    }

    async fn assert_channel_requests(server: &mut DuplexStream, client: &super::Client) {
        let mode = tokio::spawn({
            let client = client.clone();
            async move { client.mode("#papo", "+t", [] as [&str; 0]).await }
        });
        client_line(server, b"@label=3 MODE #papo +t\r\n").await;
        server_line(
            server,
            b"@label=3 :papo-driver!papo@localhost MODE #papo +t\r\n",
        )
        .await;
        mode.await.unwrap().unwrap();

        let set = tokio::spawn({
            let client = client.clone();
            async move { client.topic("#papo", Some("Requests in Rust")).await }
        });
        client_line(server, b"@label=4 TOPIC #papo :Requests in Rust\r\n").await;
        server_line(
            server,
            b"@label=4 :papo-driver!papo@localhost TOPIC #papo :Requests in Rust\r\n",
        )
        .await;
        assert_eq!(
            set.await.unwrap().unwrap().as_deref(),
            Some("Requests in Rust")
        );

        let query = tokio::spawn({
            let client = client.clone();
            async move { client.topic("#papo", None).await }
        });
        client_line(server, b"@label=5 TOPIC #papo\r\n").await;
        server_line(
            server,
            b"@label=5 :ergo.test 332 papo-driver #papo :Requests in Rust\r\n",
        )
        .await;
        assert_eq!(
            query.await.unwrap().unwrap().as_deref(),
            Some("Requests in Rust")
        );

        let part = tokio::spawn({
            let client = client.clone();
            async move { client.part("#papo", Some("tests complete")).await }
        });
        client_line(server, b"@label=6 PART #papo :tests complete\r\n").await;
        server_line(
            server,
            b"@label=6 :papo-driver!papo@localhost PART #papo :tests complete\r\n",
        )
        .await;
        part.await.unwrap().unwrap();

        let list = tokio::spawn({
            let client = client.clone();
            async move { client.list(crate::ListFilter::default()).await }
        });
        client_line(server, b"@label=7 LIST\r\n").await;
        server_line(server, b"@label=7 :ergo.test BATCH +list-42 labeled-response\r\n@batch=list-42 :ergo.test 321 papo-driver Channel :Users Name\r\n@batch=list-42 :ergo.test 322 papo-driver #papo 42 :Ergonomic IRC\r\n@batch=list-42 :ergo.test 322 papo-driver #rust 17 :The Rust Programming Language\r\n@batch=list-42 :ergo.test 323 papo-driver :End of /LIST\r\n:ergo.test BATCH -list-42\r\n").await;
        let list = list.await.unwrap().unwrap();
        assert_eq!((list.len(), list[0].name.as_str()), (2, "#papo"));
        assert_eq!(
            (list[1].members_count, list[1].topic.as_str()),
            (17, "The Rust Programming Language")
        );
    }

    async fn complete_extended_monitor_registration(
        server: &mut DuplexStream,
        events: &mut super::Events,
    ) {
        registration_burst(server).await;
        server_line(server, b":ergo.test CAP * LS :extended-monitor\r\n").await;
        client_line(server, b"CAP REQ :extended-monitor\r\n").await;
        server_line(
            server,
            b":ergo.test CAP papo-driver ACK :extended-monitor\r\n",
        )
        .await;
        client_line(server, b"CAP END\r\n").await;
        server_line(
            server,
            b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 005 papo-driver MONITOR=100 :are supported\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
    }

    async fn next_message(events: &mut super::Events) -> MessageEvent {
        let Some(Event::Message(message)) = events.next().await else {
            panic!("expected message event")
        };
        message
    }

    #[tokio::test(flavor = "current_thread")]
    async fn message_accessors_use_captured_session_facts() {
        let (mut server, _client, mut events, driver) = scripted(config().max_batch_lines(0));
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(&mut server, b":ergo.test 005 papo-driver CHANTYPES=# CASEMAPPING=rfc1459 :are supported\r\n:ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :No MOTD\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));

        server_line(&mut server, b"@time=2026-02-03T04:05:06.789Z;msgid=RfcBTzyxY120004;account=emily;bot;+draft/reply=RfcBTzyxY120003 :emily!~e@localhost PRIVMSG #papo :hello \x02from Ergo\x02\r\n").await;
        let message = next_message(&mut events).await;
        assert_eq!(message.kind(), MessageKind::Privmsg);
        assert_eq!(message.text(), "hello from Ergo");
        assert_eq!(message.raw_text(), "hello \x02from Ergo\x02");
        assert_eq!(message.target(), "#papo");
        assert!(!message.is_private());
        assert_eq!(message.reply_target(), "#papo");
        assert_eq!(message.msgid(), Some("RfcBTzyxY120004"));
        assert_eq!(message.account(), Some("emily"));
        assert!(message.is_bot());
        assert_eq!(message.reply_to(), Some("RfcBTzyxY120003"));
        assert_eq!(
            message.time(),
            UNIX_EPOCH + Duration::new(1_770_091_506, 789_000_000)
        );
        assert_eq!(message.origin(), Origin::Live);
        assert!(message.is_live());

        server_line(&mut server, b":emily!~e@localhost NOTICE papo-driver :maintenance soon\r\n:emily!~e@localhost PRIVMSG #papo :\x01ACTION waves\x01\r\n:emily!~e@localhost PRIVMSG #papo :\x01VERSION\x01\r\n").await;
        let notice = next_message(&mut events).await;
        assert_eq!(notice.kind(), MessageKind::Notice);
        assert!(notice.is_private());
        assert_eq!(notice.reply_target(), "emily");
        let action = next_message(&mut events).await;
        assert_eq!(action.kind(), MessageKind::Action);
        assert_eq!(action.text(), "waves");
        assert_eq!(
            next_message(&mut events).await.kind(),
            MessageKind::Ctcp("VERSION")
        );

        server_line(&mut server, b":PAPO-DRIVER!p@localhost PRIVMSG #papo :sent\r\nBATCH +history42 chathistory #papo\r\n@batch=history42 :dan!d@localhost PRIVMSG #papo :earlier\r\n").await;
        assert_eq!(next_message(&mut events).await.origin(), Origin::Echo);
        assert_eq!(next_message(&mut events).await.origin(), Origin::Replay);
        assert!(matches!(
            events.next().await,
            Some(Event::BatchAbandoned { .. })
        ));

        let before = SystemTime::now();
        server_line(&mut server, b":dan!d@localhost PRIVMSG #papo :no time\r\n@time=invalid :dan!d@localhost PRIVMSG #papo :bad time\r\n").await;
        let absent = next_message(&mut events).await.time();
        let malformed = next_message(&mut events).await.time();
        let after = SystemTime::now();
        assert!((before..=after).contains(&absent));
        assert!((before..=after).contains(&malformed));

        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    async fn assert_registration_output(
        defaults: super::PostRegistration,
        isupport: Option<&[u8]>,
        expected: &[u8],
    ) {
        let (client_stream, mut server) = tokio::io::duplex(4096);
        let (_client, mut events, driver) = Driver::with_stream_and_registration(
            client_stream,
            Session::new(config()),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            defaults,
        );
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
        client_line(&mut server, b"CAP END\r\n").await;
        if let Some(isupport) = isupport {
            server_line(&mut server, isupport).await;
        }
        server_line(
            &mut server,
            b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
        client_line(&mut server, expected).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(1), server.read_u8())
                .await
                .is_err()
        );
        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    fn config() -> Config {
        Config::default()
            .nick("papo-driver")
            .realname("papo driver test")
            .wanted_caps([] as [&str; 0])
    }

    #[tokio::test(flavor = "current_thread")]
    async fn latest_history_resolves_with_history_messages() {
        let config = config().wanted_caps([
            "draft/chathistory",
            "labeled-response",
            "batch",
            "server-time",
        ]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test CAP * LS :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        client_line(
            &mut server,
            b"CAP REQ :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        server_line(
            &mut server,
            b":ergo.test CAP * ACK :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(&mut server, b":ergo.test 005 papo-driver CHATHISTORY=100 MSGREFTYPES=msgid,timestamp :supported\r\n:ergo.test 001 papo-driver :Welcome\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
        let request = tokio::spawn(async move { client.history("#papo").latest(10).await });
        client_line(&mut server, b"@label=0 CHATHISTORY LATEST #papo * 10\r\n").await;
        server_line(&mut server, b"@label=0 :ergo.test BATCH +reply labeled-response\r\n@batch=reply;draft/chathistory-end :ergo.test BATCH +history chathistory #papo\r\n@batch=history;time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120004 :emily!~e@localhost PRIVMSG #papo :earlier\r\n@batch=reply :ergo.test BATCH -history\r\n:ergo.test BATCH -reply\r\n").await;
        let page = request.await.unwrap().unwrap();
        assert!(page.end);
        assert_eq!(page.messages[0].text(), "earlier");
        assert_eq!(page.messages[0].origin(), Origin::History);
        assert!(!page.messages[0].is_live());
        let Some(Event::History { messages, .. }) = events.next().await else {
            panic!("expected replay batch event")
        };
        assert_eq!(messages[0].origin(), Origin::Replay);
        task.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reference_history_queries_reach_the_wire_and_resolve() {
        let config = config().wanted_caps([
            "draft/chathistory",
            "labeled-response",
            "batch",
            "server-time",
        ]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        complete_history_registration(&mut server, &mut events).await;

        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .history("#papo")
                    .before("RfcBTzyxY119999")
                    .limit(50)
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=0 CHATHISTORY BEFORE #papo msgid=RfcBTzyxY119999 50\r\n",
        )
        .await;
        history_response(&mut server, 0).await;
        let before = request.await.unwrap().unwrap();

        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .history("#papo")
                    .after(UNIX_EPOCH + Duration::from_secs(42))
                    .limit(51)
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=1 CHATHISTORY AFTER #papo timestamp=1970-01-01T00:00:42.000Z 51\r\n",
        )
        .await;
        history_response(&mut server, 1).await;
        request.await.unwrap().unwrap();

        let message = before.messages[0].clone();
        let request = tokio::spawn({
            let client = client.clone();
            async move { client.history("#papo").around(&message).limit(52).await }
        });
        client_line(
            &mut server,
            b"@label=2 CHATHISTORY AROUND #papo msgid=RfcBTzyxY120000 52\r\n",
        )
        .await;
        history_response(&mut server, 2).await;
        request.await.unwrap().unwrap();

        let request = tokio::spawn(async move {
            client
                .history("#papo")
                .between("RfcBTzyxY119998", "RfcBTzyxY120003")
                .limit(53)
                .await
        });
        client_line(&mut server, b"@label=3 CHATHISTORY BETWEEN #papo msgid=RfcBTzyxY119998 msgid=RfcBTzyxY120003 53\r\n").await;
        history_response(&mut server, 3).await;
        request.await.unwrap().unwrap();
        task.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn history_fails_without_the_negotiated_capability() {
        let (mut server, client, mut events, driver) = scripted(config());
        let task = tokio::spawn(driver);
        complete_registration(&mut server, &mut events).await;
        assert!(matches!(
            client.history("#papo").latest(10).await,
            Err(Error::Unsupported("chathistory"))
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(1), server.read_u8())
                .await
                .is_err()
        );
        task.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn history_targets_are_typed_and_time_bounded() {
        let config = config().wanted_caps([
            "draft/chathistory",
            "labeled-response",
            "batch",
            "server-time",
        ]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        complete_history_registration(&mut server, &mut events).await;
        let request = tokio::spawn(async move {
            client
                .history_targets(
                    UNIX_EPOCH + Duration::from_secs(42),
                    UNIX_EPOCH + Duration::from_secs(84),
                )
                .await
        });
        client_line(&mut server, b"@label=0 CHATHISTORY TARGETS 1970-01-01T00:00:42.000Z 1970-01-01T00:01:24.000Z 100\r\n").await;
        server_line(&mut server, b"@label=0 :ergo.test BATCH +reply labeled-response\r\n@batch=reply :ergo.test BATCH +targets draft/chathistory-targets\r\n@batch=targets :ergo.test CHATHISTORY TARGETS #papo 2026-10-15T12:00:00.000Z\r\n@batch=reply :ergo.test BATCH -targets\r\n:ergo.test BATCH -reply\r\n").await;
        let targets = request.await.unwrap().unwrap();
        assert_eq!(
            targets,
            [("#papo".into(), UNIX_EPOCH + Duration::from_hours(497_796))]
        );
        task.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mark_read_sets_a_millisecond_server_timestamp() {
        let config = config().wanted_caps(["draft/read-marker", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        complete_read_marker_registration(&mut server, &mut events).await;
        let request = tokio::spawn(async move {
            client
                .mark_read(
                    "#papo",
                    UNIX_EPOCH + Duration::new(1_760_529_342, 123_999_999),
                )
                .await
        });
        client_line(
            &mut server,
            b"@label=0 MARKREAD #papo timestamp=2025-10-15T11:55:42.123Z\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=0 :ergo.test MARKREAD #papo timestamp=2025-10-15T11:55:42.123Z\r\n",
        )
        .await;
        request.await.unwrap().unwrap();
        assert!(
            matches!(events.next().await, Some(Event::ReadMarker(marker)) if marker.target == "#papo")
        );
        task.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn read_marker_resolves_fresh_and_cached_values() {
        let config = config().wanted_caps(["draft/read-marker", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        complete_read_marker_registration(&mut server, &mut events).await;
        let request = tokio::spawn({
            let client = client.clone();
            async move { client.read_marker("#papo").await }
        });
        client_line(&mut server, b"MARKREAD #papo\r\n").await;
        server_line(
            &mut server,
            b":ergo.test MARKREAD #papo timestamp=2025-10-15T11:55:42.123Z\r\n",
        )
        .await;
        let expected = UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000);
        assert_eq!(request.await.unwrap().unwrap(), Some(expected));
        assert_eq!(
            client.state(|state| state.read_marker("#PAPO")),
            Some(expected)
        );

        assert_eq!(client.read_marker("#papo").await.unwrap(), Some(expected));
        client_line(&mut server, b"MARKREAD #papo\r\n").await;
        task.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn registration_is_forwarded() {
        let (mut server, client, mut events, driver) = scripted(config());
        assert!(!client.is_connected());
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        assert!(client.is_connected());
        server
            .write_all(b":ergo.test CAP * LS :\r\n")
            .await
            .unwrap();
        let mut cap_end = [0; 9];
        server.read_exact(&mut cap_end).await.unwrap();
        assert_eq!(&cap_end, b"CAP END\r\n");
        server
            .write_all(b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n")
            .await
            .unwrap();
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
        assert!(!client.is_connected());
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn connection_failures_retry_up_to_the_configured_limit() {
        let calls = Arc::new(AtomicUsize::new(0));
        let connector: super::Connector = Arc::new({
            let calls = Arc::clone(&calls);
            move |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                Box::pin(async { Err(Error::Unsupported("scripted connection failure")) })
            }
        });
        let (_client, mut events, driver) = Driver::with_connector_and_entropy(
            Ok(ConnectionPlan {
                address: "irc://irc.example.net".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(config()),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration::default(),
            crate::Reconnect::exponential().max_attempts(2),
            256,
            Box::new(papo_core::sts::MemoryStsStore::new()),
            false,
            connector,
            Box::new(|| 0),
        );
        let driver = tokio::spawn(driver);
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                will_retry: true,
                ..
            })
        ));
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                will_retry: false,
                ..
            })
        ));
        assert!(matches!(driver.await.unwrap(), Err(Error::Unsupported(_))));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn eof_reconnects_and_emits_reconnected() {
        let (first_stream, mut first_server) = tokio::io::duplex(4096);
        let (second_stream, mut second_server) = tokio::io::duplex(4096);
        let streams = Arc::new(Mutex::new(VecDeque::from([
            Box::new(first_stream) as super::BoxConnection,
            Box::new(second_stream) as super::BoxConnection,
        ])));
        let connector: super::Connector = Arc::new(move |_| {
            let stream = streams.lock().unwrap().pop_front().unwrap();
            Box::pin(async move { Ok(stream) })
        });
        let (client, mut events, driver) = Driver::with_connector(
            Ok(ConnectionPlan {
                address: "irc://irc.example.net".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(config().wanted_caps([
                "draft/chathistory",
                "labeled-response",
                "batch",
                "server-time",
            ])),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration {
                bot: false,
                channels: vec!["#papo".into()],
                history_on_reconnect: false,
            },
            Reconnect::exponential(),
            256,
            Box::new(papo_core::sts::MemoryStsStore::new()),
            false,
            connector,
        );
        let driver = tokio::spawn(driver);

        complete_history_registration(&mut first_server, &mut events).await;
        client_line(&mut first_server, b"JOIN #papo\r\n").await;
        server_line(&mut first_server, b"@time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120000 :emily!~e@localhost PRIVMSG #papo :before disconnect\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Message(_))));
        assert!(client.is_connected());
        first_server.shutdown().await.unwrap();
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                will_retry: true,
                ..
            })
        ));
        assert!(!client.is_connected());

        tokio::time::advance(Duration::from_secs(2)).await;
        registration_burst(&mut second_server).await;
        server_line(
            &mut second_server,
            b":ergo.test CAP * LS :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        client_line(
            &mut second_server,
            b"CAP REQ :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        server_line(
            &mut second_server,
            b":ergo.test CAP * ACK :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        client_line(&mut second_server, b"CAP END\r\n").await;
        server_line(
            &mut second_server,
            b":ergo.test 005 papo-driver CHATHISTORY=100 MSGREFTYPES=msgid,timestamp :supported\r\n:ergo.test 001 papo-driver :Welcome back\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::Reconnected(_))));
        client_line(&mut second_server, b"JOIN #papo\r\n").await;
        assert!(
            tokio::time::timeout(Duration::from_millis(1), second_server.read_u8())
                .await
                .is_err()
        );
        assert!(client.is_connected());

        let quit = tokio::spawn(async move { client.quit(None).await });
        client_line(&mut second_server, b"QUIT\r\n").await;
        second_server.shutdown().await.unwrap();
        assert!(quit.await.unwrap().is_ok());
        assert!(driver.await.unwrap().is_ok());
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn reconnect_history_pages_deduplicate_into_one_non_live_event() {
        let (first_stream, mut first_server) = tokio::io::duplex(4096);
        let (second_stream, mut second_server) = tokio::io::duplex(4096);
        let streams = Arc::new(Mutex::new(VecDeque::from([
            Box::new(first_stream) as super::BoxConnection,
            Box::new(second_stream) as super::BoxConnection,
        ])));
        let connector: super::Connector = Arc::new(move |_| {
            let stream = streams.lock().unwrap().pop_front().unwrap();
            Box::pin(async move { Ok(stream) })
        });
        let config = config().wanted_caps([
            "draft/chathistory",
            "labeled-response",
            "batch",
            "server-time",
        ]);
        let (_client, mut events, driver) = Driver::with_connector(
            Ok(ConnectionPlan {
                address: "irc://irc.example.net".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(config),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration {
                bot: false,
                channels: vec!["#papo".into()],
                history_on_reconnect: true,
            },
            Reconnect::exponential(),
            256,
            Box::new(papo_core::sts::MemoryStsStore::new()),
            false,
            connector,
        );
        let driver = tokio::spawn(driver);

        complete_history_registration(&mut first_server, &mut events).await;
        client_line(&mut first_server, b"JOIN #papo\r\n").await;
        server_line(&mut first_server, b"@time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120000 :emily!~e@localhost PRIVMSG #papo :before disconnect\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Message(_))));
        first_server.shutdown().await.unwrap();
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected { .. })
        ));
        tokio::time::advance(Duration::from_secs(2)).await;

        registration_burst(&mut second_server).await;
        server_line(
            &mut second_server,
            b":ergo.test CAP * LS :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        client_line(
            &mut second_server,
            b"CAP REQ :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        server_line(
            &mut second_server,
            b":ergo.test CAP * ACK :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        client_line(&mut second_server, b"CAP END\r\n").await;
        server_line(&mut second_server, b":ergo.test 005 papo-driver CHATHISTORY=100 MSGREFTYPES=msgid,timestamp :supported\r\n:ergo.test 001 papo-driver :Welcome back\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Reconnected(_))));
        client_line(&mut second_server, b"JOIN #papo\r\n").await;
        client_line(
            &mut second_server,
            b"@label=0 CHATHISTORY AFTER #papo msgid=RfcBTzyxY120000 100\r\n",
        )
        .await;
        server_line(&mut second_server, b"@label=0 :ergo.test BATCH +reply0 labeled-response\r\n@batch=reply0 :ergo.test BATCH +history0 chathistory #papo\r\n@batch=history0;time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120000 :emily!~e@localhost PRIVMSG #papo :before disconnect\r\n@batch=history0;time=2026-10-15T12:01:00.000Z;msgid=RfcBTzyxY120001 :emily!~e@localhost PRIVMSG #papo :missed one\r\n@batch=reply0 :ergo.test BATCH -history0\r\n:ergo.test BATCH -reply0\r\n").await;
        client_line(
            &mut second_server,
            b"@label=1 CHATHISTORY AFTER #papo msgid=RfcBTzyxY120001 100\r\n",
        )
        .await;
        server_line(&mut second_server, b"@label=1 :ergo.test BATCH +reply1 labeled-response\r\n@batch=reply1;draft/chathistory-end :ergo.test BATCH +history1 chathistory #papo\r\n@batch=history1;time=2026-10-15T12:01:00.000Z;msgid=RfcBTzyxY120001 :emily!~e@localhost PRIVMSG #papo :missed one\r\n@batch=history1;time=2026-10-15T12:02:00.000Z;msgid=RfcBTzyxY120002 :emily!~e@localhost PRIVMSG #papo :missed two\r\n@batch=reply1 :ergo.test BATCH -history1\r\n:ergo.test BATCH -reply1\r\n").await;

        let Some(Event::History {
            target,
            messages,
            end,
        }) = events.next().await
        else {
            panic!("expected aggregate history event")
        };
        assert_eq!(target, "#papo");
        assert_eq!(
            messages.iter().map(MessageEvent::text).collect::<Vec<_>>(),
            ["missed one", "missed two"]
        );
        assert!(
            messages
                .iter()
                .all(|message| message.origin() == Origin::History && !message.is_live())
        );
        assert!(end);
        driver.abort();
    }

    #[test]
    fn reconnect_history_stops_at_the_literal_per_target_limit() {
        for count in [1_000, 1_001] {
            let mut catchup = super::Catchup {
                target: "#papo".into(),
                messages: Vec::new(),
                seen: std::collections::HashSet::new(),
            };
            let next = super::extend_catchup(
                &mut catchup,
                (0..count).map(|index| {
                    Message::privmsg("#papo", format!("backlog message {index}"))
                        .with_tag("msgid", format!("RfcBTzyxY12{index:04}"))
                }),
            );
            assert_eq!(catchup.messages.len(), 1_000);
            assert_eq!(next.as_deref(), Some("RfcBTzyxY120999"));
        }
    }

    #[test]
    fn persistence_gates_rejoin_and_history_fetch() {
        for (effective, expected) in [(false, true), (true, false)] {
            let now = std::time::Instant::now();
            let mut session = Session::new(config().wanted_caps(["draft/persistence"]));
            session.handle_connected(now);
            while session.poll_transmit().is_some() {}
            session.handle_bytes(now, b":ergo.test CAP * LS :draft/persistence\r\n");
            while session.poll_transmit().is_some() {}
            session.handle_bytes(
                now,
                b":ergo.test CAP * ACK :draft/persistence\r\n:ergo.test PERSISTENCE STATUS DEFAULT ",
            );
            session.handle_bytes(now, if effective { b"ON\r\n" } else { b"OFF\r\n" });
            assert_eq!(super::should_rejoin(&session), expected);
            assert_eq!(super::should_fetch_history(&session), expected);
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn commands_queue_while_disconnected_and_overflow_is_rejected() {
        let (client_stream, mut server) = tokio::io::duplex(4096);
        let results = Arc::new(Mutex::new(VecDeque::from([
            Err(Error::Unsupported("scripted connection failure")),
            Ok(Box::new(client_stream) as super::BoxConnection),
        ])));
        let connector: super::Connector = Arc::new(move |_| {
            let result = results.lock().unwrap().pop_front().unwrap();
            Box::pin(async move { result })
        });
        let (client, mut events, driver) = Driver::with_connector(
            Ok(ConnectionPlan {
                address: "irc://irc.example.net".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(config()),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration::default(),
            Reconnect::exponential(),
            3,
            Box::new(papo_core::sts::MemoryStsStore::new()),
            false,
            connector,
        );
        let driver = tokio::spawn(driver);
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                will_retry: true,
                ..
            })
        ));

        client.send(Message::new("WHOIS", ["emily"])).await.unwrap();
        client
            .privmsg("#papo", "queued during reconnect")
            .await
            .unwrap();
        client.send(Message::new("MODE", ["#papo"])).await.unwrap();
        assert!(matches!(
            client.privmsg("#papo", "queue overflow").await,
            Err(Error::Disconnected)
        ));

        tokio::time::advance(Duration::from_secs(2)).await;
        registration_burst(&mut server).await;
        client_line(&mut server, b"WHOIS emily\r\n").await;
        client_line(&mut server, b"PRIVMSG #papo :queued during reconnect\r\n").await;
        client_line(&mut server, b"MODE #papo\r\n").await;
        let quit = tokio::spawn(async move { client.quit(None).await });
        client_line(&mut server, b"QUIT\r\n").await;
        server.shutdown().await.unwrap();
        assert!(quit.await.unwrap().is_ok());
        assert!(driver.await.unwrap().is_ok());
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn quit_while_disconnected_stops_without_another_attempt() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let connector: super::Connector = Arc::new({
            let calls = Arc::clone(&calls);
            move |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                Box::pin(async { Err(Error::Unsupported("scripted connection failure")) })
            }
        });
        let (client, mut events, driver) = Driver::with_connector(
            Ok(ConnectionPlan {
                address: "irc://irc.example.net".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(config()),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration::default(),
            Reconnect::exponential(),
            256,
            Box::new(papo_core::sts::MemoryStsStore::new()),
            false,
            connector,
        );
        let driver = tokio::spawn(driver);
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                will_retry: true,
                ..
            })
        ));
        client.quit(Some("cancel reconnect")).await.unwrap();
        assert!(driver.await.unwrap().is_ok());
        tokio::time::advance(Duration::from_secs(10)).await;
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn failed_connects_back_off_until_the_attempt_limit() {
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let connector: super::Connector = Arc::new({
            let attempts = Arc::clone(&attempts);
            move |_| {
                attempts.lock().unwrap().push(tokio::time::Instant::now());
                Box::pin(async { Err(Error::Unsupported("scripted connection failure")) })
            }
        });
        let (_client, mut events, driver) = Driver::with_connector_and_entropy(
            Ok(ConnectionPlan {
                address: "irc://irc.example.net".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(config()),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration::default(),
            Reconnect::exponential().max_attempts(3),
            256,
            Box::new(papo_core::sts::MemoryStsStore::new()),
            false,
            connector,
            Box::new(|| 0),
        );
        let driver = tokio::spawn(driver);

        for expected in [true, true, false] {
            assert!(matches!(
                events.next().await,
                Some(Event::Disconnected { will_retry, .. }) if will_retry == expected
            ));
        }
        assert!(matches!(
            driver.await.unwrap(),
            Err(Error::Unsupported("scripted connection failure"))
        ));
        let attempts = attempts.lock().unwrap();
        assert_eq!(attempts.len(), 3);
        assert_eq!(attempts[1] - attempts[0], Duration::from_millis(800));
        assert_eq!(attempts[2] - attempts[1], Duration::from_millis(1600));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn stable_registration_resets_the_backoff() {
        let (stable_stream, mut stable_server) = tokio::io::duplex(4096);
        let (next_stream, mut next_server) = tokio::io::duplex(4096);
        let results = Arc::new(Mutex::new(VecDeque::from([
            Err(Error::Unsupported("first scripted failure")),
            Err(Error::Unsupported("second scripted failure")),
            Ok(Box::new(stable_stream) as super::BoxConnection),
            Ok(Box::new(next_stream) as super::BoxConnection),
        ])));
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let connector: super::Connector = Arc::new({
            let attempts = Arc::clone(&attempts);
            let results = Arc::clone(&results);
            move |_| {
                attempts.lock().unwrap().push(tokio::time::Instant::now());
                let result = results.lock().unwrap().pop_front().unwrap();
                Box::pin(async move { result })
            }
        });
        let (client, mut events, driver) = Driver::with_connector_and_entropy(
            Ok(ConnectionPlan {
                address: "irc://irc.example.net".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(config()),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration::default(),
            Reconnect::exponential(),
            256,
            Box::new(papo_core::sts::MemoryStsStore::new()),
            false,
            connector,
            Box::new(|| 0),
        );
        let driver = tokio::spawn(driver);
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected { .. })
        ));
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected { .. })
        ));

        registration_burst(&mut stable_server).await;
        server_line(&mut stable_server, b":ergo.test CAP * LS :\r\n").await;
        client_line(&mut stable_server, b"CAP END\r\n").await;
        server_line(
            &mut stable_server,
            b":ergo.test 001 papo-driver :Welcome back\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
        tokio::time::advance(Duration::from_secs(60)).await;
        let disconnected_at = tokio::time::Instant::now();
        stable_server.shutdown().await.unwrap();
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected { .. })
        ));

        registration_burst(&mut next_server).await;
        let delay = {
            let attempts = attempts.lock().unwrap();
            assert_eq!(attempts.len(), 4);
            attempts[3] - disconnected_at
        };
        assert_eq!(delay, Duration::from_millis(800));
        let quit = tokio::spawn(async move { client.quit(None).await });
        client_line(&mut next_server, b"QUIT\r\n").await;
        next_server.shutdown().await.unwrap();
        assert!(quit.await.unwrap().is_ok());
        assert!(driver.await.unwrap().is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stored_sts_policy_upgrades_the_connection_plan() {
        let (client_stream, mut server) = tokio::io::duplex(4096);
        let stream = Arc::new(Mutex::new(Some(
            Box::new(client_stream) as super::BoxConnection
        )));
        let connector: super::Connector = Arc::new(move |plan| {
            assert!(plan.address.is_secure());
            assert_eq!(plan.address.port, 7443);
            let stream = stream.lock().unwrap().take().unwrap();
            Box::pin(async move { Ok(stream) })
        });
        let mut store = papo_core::sts::MemoryStsStore::new();
        papo_core::sts::StsStore::put(
            &mut store,
            papo_core::sts::StoredPolicy {
                host: "irc.example.net".into(),
                port: 7443,
                expires_at: std::time::SystemTime::now() + Duration::from_secs(3600),
                duration: Duration::from_secs(3600),
            },
        );
        let (client, _events, driver) = Driver::with_connector(
            Ok(ConnectionPlan {
                address: "irc://irc.example.net".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(config()),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration::default(),
            Reconnect::exponential(),
            256,
            Box::new(store),
            false,
            connector,
        );
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        let quit = tokio::spawn(async move { client.quit(None).await });
        client_line(&mut server, b"QUIT\r\n").await;
        server.shutdown().await.unwrap();
        assert!(quit.await.unwrap().is_ok());
        assert!(driver.await.unwrap().is_ok());
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn connector_failure_under_stored_sts_is_terminal() {
        let calls = Arc::new(AtomicUsize::new(0));
        let connector: super::Connector = Arc::new({
            let calls = Arc::clone(&calls);
            move |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                Box::pin(async { Err(Error::Unsupported("scripted TLS failure")) })
            }
        });
        let mut store = papo_core::sts::MemoryStsStore::new();
        papo_core::sts::StsStore::put(
            &mut store,
            papo_core::sts::StoredPolicy {
                host: "irc.example.net".into(),
                port: 6697,
                expires_at: SystemTime::now() + Duration::from_secs(3600),
                duration: Duration::from_secs(3600),
            },
        );
        let (_client, mut events, driver) = Driver::with_connector(
            Ok(ConnectionPlan {
                address: "irc://irc.example.net".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(config()),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration::default(),
            Reconnect::exponential(),
            256,
            Box::new(store),
            false,
            connector,
        );

        assert!(matches!(
            driver.await,
            Err(Error::Unsupported("scripted TLS failure"))
        ));
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                will_retry: false,
                ..
            })
        ));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn eof_emits_disconnected_and_returns_an_error() {
        let (mut server, _client, mut events, driver) = scripted(config());
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server.shutdown().await.unwrap();
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                reason,
                will_retry: false,
            }) if reason == "server closed the connection"
        ));
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_error_overrides_a_non_io_terminal_cause() {
        let (mut server, _client, mut events, driver) = scripted_with_failures(false);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server.shutdown().await.unwrap();
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                reason,
                will_retry: false,
            }) if reason == "shutdown failed"
        ));
        assert!(matches!(driver.await.unwrap(), Err(Error::Io(error))
            if error.to_string() == "shutdown failed"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_error_does_not_replace_an_io_terminal_cause() {
        let (mut server, _client, mut events, driver) = scripted_with_failures(true);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                reason,
                will_retry: false,
            }) if reason == "original read failure"
        ));
        assert!(matches!(driver.await.unwrap(), Err(Error::Io(error))
            if error.to_string() == "original read failure"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn overlong_line_becomes_a_terminal_disconnect() {
        let (mut server, _client, mut events, driver) = scripted(config());
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server.write_all(&vec![b'x'; 8704]).await.unwrap();
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                reason,
                will_retry: false,
            }) if reason == "incoming IRC line exceeds 8703 bytes"
        ));
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn malformed_line_preserves_exact_bytes() {
        let (mut server, _client, mut events, driver) = scripted(config().decoding(Decoding::Utf8));
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server
            .write_all(b":ergo.test NOTICE * :caf\xe9\r\n")
            .await
            .unwrap();
        assert!(matches!(
            events.next().await,
            Some(Event::Malformed { line }) if line == b":ergo.test NOTICE * :caf\xe9"
        ));
        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sasl_nak_forwards_authentication_failure() {
        let settings = config()
            .wanted_caps(["sasl"])
            .sasl(Sasl::plain("alice", "correct horse battery staple"))
            .require_sasl(false);
        let (mut server, _client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :sasl=PLAIN\r\n").await;
        client_line(&mut server, b"CAP REQ :sasl\r\n").await;
        server_line(&mut server, b":ergo.test CAP * NAK :sasl\r\n").await;
        assert!(matches!(
            events.next().await,
            Some(Event::AuthenticationFailed { .. })
        ));
        client_line(&mut server, b"CAP END\r\n").await;
        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn required_sasl_failure_rejects_registration_waiter() {
        let settings = config()
            .wanted_caps(["sasl"])
            .sasl(Sasl::plain("alice", "correct horse battery staple"))
            .require_sasl(true);
        let (client_stream, mut server) = tokio::io::duplex(4096);
        let (registration, waiter) = tokio::sync::oneshot::channel();
        let (_client, _events, driver) = Driver::with_stream_and_registration(
            client_stream,
            Session::new(settings),
            super::DEFAULT_CHANNEL_CAPACITY,
            Some(registration),
            super::PostRegistration::default(),
        );
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :sasl=PLAIN\r\n").await;
        client_line(&mut server, b"CAP REQ :sasl\r\n").await;
        server_line(&mut server, b":ergo.test CAP * NAK :sasl\r\n").await;

        assert!(
            matches!(waiter.await.unwrap(), Err(Error::Registration(reason)) if reason.contains("SASL"))
        );
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn required_sasl_failure_does_not_reconnect() {
        let settings = config()
            .wanted_caps(["sasl"])
            .sasl(Sasl::plain("alice", "correct horse battery staple"))
            .require_sasl(true);
        let (client_stream, mut server) = tokio::io::duplex(4096);
        let stream = Arc::new(Mutex::new(Some(
            Box::new(client_stream) as super::BoxConnection
        )));
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let connector: super::Connector = Arc::new({
            let calls = Arc::clone(&calls);
            move |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                let stream = stream.lock().unwrap().take().unwrap();
                Box::pin(async move { Ok(stream) })
            }
        });
        let (_client, mut events, driver) = Driver::with_connector(
            Ok(ConnectionPlan {
                address: "irc://irc.example.net".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(settings),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration::default(),
            Reconnect::exponential(),
            256,
            Box::new(papo_core::sts::MemoryStsStore::new()),
            false,
            connector,
        );
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :sasl=PLAIN\r\n").await;
        client_line(&mut server, b"CAP REQ :sasl\r\n").await;
        server_line(&mut server, b":ergo.test CAP * NAK :sasl\r\n").await;

        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                will_retry: false,
                ..
            })
        ));
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
        tokio::time::advance(Duration::from_secs(10)).await;
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[cfg(tls)]
    #[test]
    fn certificate_verification_failure_is_fatal() {
        let error = Error::Connect(crate::transport::ConnectError::Tls(
            rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
        ));
        assert!(super::is_fatal_failure(&error, None));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn successful_registration_preserves_earlier_events() {
        let (client_stream, mut server) = tokio::io::duplex(4096);
        let (registration, waiter) = tokio::sync::oneshot::channel();
        let (_client, mut events, driver) = Driver::with_stream_and_registration(
            client_stream,
            Session::new(config()),
            super::DEFAULT_CHANNEL_CAPACITY,
            Some(registration),
            super::PostRegistration::default(),
        );
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(
            &mut server,
            b":ergo.test NOTICE * :*** Looking up your hostname...\r\n:ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 376 papo-driver :End of MOTD\r\n",
        )
        .await;

        assert!(waiter.await.unwrap().is_ok());
        assert!(matches!(
            events.next().await,
            Some(Event::Message(message)) if message.as_ref().command_is("NOTICE")
        ));
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn registration_sets_advertised_bot_mode_and_joins_each_channel() {
        let (client_stream, mut server) = tokio::io::duplex(4096);
        let (_client, mut events, driver) = Driver::with_stream_and_registration(
            client_stream,
            Session::new(config()),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration {
                bot: true,
                channels: vec!["#papo".into(), "#papo-bots".into()],
                history_on_reconnect: false,
            },
        );
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(
            &mut server,
            b":ergo.test 005 papo-driver BOT=B :are supported by this server\r\n:ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
        client_line(
            &mut server,
            b"MODE papo-driver +B\r\nJOIN #papo\r\nJOIN #papo-bots\r\n",
        )
        .await;
        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bot_default_without_advertised_mode_sends_no_mode() {
        assert_registration_output(
            super::PostRegistration {
                bot: true,
                channels: Vec::new(),
                history_on_reconnect: false,
            },
            None,
            b"",
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn advertised_bot_mode_without_bot_default_sends_no_mode() {
        assert_registration_output(
            super::PostRegistration::default(),
            Some(b":ergo.test 005 papo-driver BOT=B :are supported by this server\r\n"),
            b"",
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn empty_channel_defaults_send_no_join() {
        assert_registration_output(
            super::PostRegistration {
                bot: true,
                channels: Vec::new(),
                history_on_reconnect: false,
            },
            Some(b":ergo.test 005 papo-driver BOT=B :are supported by this server\r\n"),
            b"MODE papo-driver +B\r\n",
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cap_new_and_del_forward_capability_changes() {
        let (mut server, _client, mut events, driver) =
            scripted(config().wanted_caps(["server-time"]));
        let driver = tokio::spawn(driver);
        complete_registration(&mut server, &mut events).await;
        server_line(
            &mut server,
            b":ergo.test CAP papo-driver NEW :server-time=2027\r\n",
        )
        .await;
        client_line(&mut server, b"CAP REQ :server-time\r\n").await;
        server_line(
            &mut server,
            b":ergo.test CAP papo-driver ACK :server-time\r\n",
        )
        .await;
        assert!(
            matches!(events.next().await, Some(Event::CapsChanged { added, .. })
            if added.get("server-time") == Some(&Some("2027".to_owned())))
        );
        server_line(
            &mut server,
            b":ergo.test CAP papo-driver DEL :server-time\r\n",
        )
        .await;
        assert!(
            matches!(events.next().await, Some(Event::CapsChanged { removed, .. })
            if removed.contains_key("server-time"))
        );
        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn plaintext_sts_forwards_upgrade_and_closes() {
        let settings = config().host("irc.example.net").wanted_caps(["sts"]);
        let (mut server, _client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :sts=port=6697\r\n").await;
        assert!(matches!(
            events.next().await,
            Some(Event::StsUpgrade { port: 6697 })
        ));
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                reason,
                will_retry: false,
            }) if reason == "sts upgrade"
        ));
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn plaintext_sts_reconnects_immediately_with_tls_on_the_advertised_port() {
        let (first_stream, mut first_server) = tokio::io::duplex(4096);
        let (second_stream, mut second_server) = tokio::io::duplex(4096);
        let streams = Arc::new(Mutex::new(VecDeque::from([
            Box::new(first_stream) as super::BoxConnection,
            Box::new(second_stream) as super::BoxConnection,
        ])));
        let requested = Arc::new(Mutex::new(Vec::new()));
        let connector: super::Connector = Arc::new({
            let requested = Arc::clone(&requested);
            move |plan| {
                requested
                    .lock()
                    .unwrap()
                    .push((plan.address.is_secure(), plan.address.port));
                let stream = streams.lock().unwrap().pop_front().unwrap();
                Box::pin(async move { Ok(stream) })
            }
        });
        let (client, mut events, driver) = Driver::with_connector(
            Ok(ConnectionPlan {
                address: "irc://irc.example.net".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(config().host("irc.example.net").wanted_caps(["sts"])),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration::default(),
            Reconnect::exponential(),
            256,
            Box::new(papo_core::sts::MemoryStsStore::new()),
            false,
            connector,
        );
        let driver = tokio::spawn(driver);
        registration_burst(&mut first_server).await;
        server_line(&mut first_server, b":ergo.test CAP * LS :sts=port=6699\r\n").await;
        assert!(matches!(
            events.next().await,
            Some(Event::StsUpgrade { port: 6699 })
        ));
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                will_retry: true,
                ..
            })
        ));
        registration_burst(&mut second_server).await;
        assert_eq!(&*requested.lock().unwrap(), &[(false, 6667), (true, 6699)]);

        let quit = tokio::spawn(async move { client.quit(None).await });
        client_line(&mut second_server, b"QUIT\r\n").await;
        second_server.shutdown().await.unwrap();
        assert!(quit.await.unwrap().is_ok());
        assert!(driver.await.unwrap().is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sts_upgrade_obeys_the_attempt_limit() {
        let (stream, mut server) = tokio::io::duplex(4096);
        let stream = Arc::new(Mutex::new(Some(Box::new(stream) as super::BoxConnection)));
        let connector: super::Connector = Arc::new(move |_| {
            let stream = stream.lock().unwrap().take().unwrap();
            Box::pin(async move { Ok(stream) })
        });
        let (_client, mut events, driver) = Driver::with_connector(
            Ok(ConnectionPlan {
                address: "irc://irc.example.net".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(config().host("irc.example.net").wanted_caps(["sts"])),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration::default(),
            Reconnect::exponential().max_attempts(1),
            256,
            Box::new(papo_core::sts::MemoryStsStore::new()),
            false,
            connector,
        );
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :sts=port=6697\r\n").await;
        assert!(matches!(
            events.next().await,
            Some(Event::StsUpgrade { .. })
        ));
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                will_retry: false,
                ..
            })
        ));
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[test]
    fn consecutive_sts_upgrades_are_a_typed_error() {
        let mut last_host = None;
        assert!(super::record_sts_upgrade(&mut last_host, "irc.example.net").is_ok());
        assert!(matches!(
            super::record_sts_upgrade(&mut last_host, "irc.example.net"),
            Err(Error::StsUpgradeLoop { host }) if host == "irc.example.net"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn secure_sts_forwards_persistence_policy() {
        let settings = config()
            .connection_security(ConnectionSecurity::Secure)
            .host("IRC.Example.NET.")
            .wanted_caps(["sts"]);
        let (mut server, _client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test CAP * LS :sts=duration=3600,preload\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::StsPolicy(
            StsAction::Persist { host, duration }
        )) if host == "irc.example.net" && duration == Duration::from_secs(3600)));
        client_line(&mut server, b"CAP END\r\n").await;
        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn secure_sts_persists_the_duration_and_connection_port() {
        let (client_stream, mut server) = tokio::io::duplex(4096);
        let stream = Arc::new(Mutex::new(Some(
            Box::new(client_stream) as super::BoxConnection
        )));
        let connector: super::Connector = Arc::new(move |_| {
            let stream = stream.lock().unwrap().take().unwrap();
            Box::pin(async move { Ok(stream) })
        });
        let store = SharedStsStore::default();
        let mut inspection = store.clone();
        let (_client, mut events, driver) = Driver::with_connector(
            Ok(ConnectionPlan {
                address: "ircs://irc.example.net:7443".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(
                config()
                    .connection_security(ConnectionSecurity::Secure)
                    .host("irc.example.net")
                    .wanted_caps(["sts"]),
            ),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration::default(),
            Reconnect::never(),
            256,
            Box::new(store),
            false,
            connector,
        );
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :sts=duration=3600\r\n").await;
        assert!(matches!(events.next().await, Some(Event::StsPolicy(_))));
        let mut policy = papo_core::sts::StsStore::get(&inspection, "irc.example.net").unwrap();
        assert_eq!(policy.port, 7443);
        assert_eq!(policy.duration, Duration::from_secs(3600));
        assert!(policy.expires_at > std::time::SystemTime::now());
        policy.expires_at = SystemTime::UNIX_EPOCH;
        papo_core::sts::StsStore::put(&mut inspection, policy);
        client_line(&mut server, b"CAP END\r\n").await;
        let disconnect_started = SystemTime::now();
        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
        let disconnect_finished = SystemTime::now();
        let refreshed = papo_core::sts::StsStore::get(&inspection, "irc.example.net").unwrap();
        assert!(
            refreshed.expires_at
                >= disconnect_started
                    .checked_add(Duration::from_secs(3600))
                    .unwrap()
        );
        assert!(
            refreshed.expires_at
                <= disconnect_finished
                    .checked_add(Duration::from_secs(3600))
                    .unwrap()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn secure_zero_duration_sts_removes_the_stored_policy() {
        let (client_stream, mut server) = tokio::io::duplex(4096);
        let stream = Arc::new(Mutex::new(Some(
            Box::new(client_stream) as super::BoxConnection
        )));
        let connector: super::Connector = Arc::new(move |_| {
            let stream = stream.lock().unwrap().take().unwrap();
            Box::pin(async move { Ok(stream) })
        });
        let mut store = SharedStsStore::default();
        papo_core::sts::StsStore::put(
            &mut store,
            papo_core::sts::StoredPolicy {
                host: "irc.example.net".to_owned(),
                port: 6697,
                expires_at: SystemTime::now() + Duration::from_secs(3600),
                duration: Duration::from_secs(3600),
            },
        );
        let inspection = store.clone();
        let (_client, mut events, driver) = Driver::with_connector(
            Ok(ConnectionPlan {
                address: "ircs://irc.example.net:6697".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(
                config()
                    .connection_security(ConnectionSecurity::Secure)
                    .host("irc.example.net")
                    .wanted_caps(["sts"]),
            ),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration::default(),
            Reconnect::never(),
            256,
            Box::new(store),
            false,
            connector,
        );
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :sts=duration=0\r\n").await;
        assert!(matches!(
            events.next().await,
            Some(Event::StsPolicy(StsAction::Remove { .. }))
        ));
        assert!(papo_core::sts::StsStore::get(&inspection, "irc.example.net").is_none());
        client_line(&mut server, b"CAP END\r\n").await;
        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn secure_sts_is_stored_before_a_saturated_event_queue_drains() {
        let (client_stream, mut server) = tokio::io::duplex(4096);
        let stream = Arc::new(Mutex::new(Some(
            Box::new(client_stream) as super::BoxConnection
        )));
        let connector: super::Connector = Arc::new(move |_| {
            let stream = stream.lock().unwrap().take().unwrap();
            Box::pin(async move { Ok(stream) })
        });
        let store = SharedStsStore::default();
        let inspection = store.clone();
        let (_client, mut events, driver) = Driver::with_connector(
            Ok(ConnectionPlan {
                address: "ircs://irc.example.net:7443".parse().unwrap(),
                tls: TlsConfig::default(),
            }),
            Session::new(
                config()
                    .connection_security(ConnectionSecurity::Secure)
                    .host("irc.example.net")
                    .wanted_caps(["sts"]),
            ),
            1,
            None,
            super::PostRegistration::default(),
            Reconnect::never(),
            256,
            Box::new(store),
            false,
            connector,
        );
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test NOTICE papo-driver :maintenance soon\r\n",
        )
        .await;
        while events.receiver.len() != 1 {
            tokio::task::yield_now().await;
        }

        server_line(&mut server, b":ergo.test CAP * LS :sts=duration=3600\r\n").await;
        tokio::time::timeout(Duration::from_secs(1), async {
            while papo_core::sts::StsStore::get(&inspection, "irc.example.net").is_none() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(events.receiver.len(), 1);
        assert!(matches!(events.next().await, Some(Event::Message(_))));
        assert!(matches!(events.next().await, Some(Event::StsPolicy(_))));
        client_line(&mut server, b"CAP END\r\n").await;
        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_send_reaches_the_wire() {
        let (mut server, client, _events, driver) = scripted(config());
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        client
            .raw_unlabeled(papo_proto::Message::new("WHOIS", ["emily"]))
            .await
            .unwrap();
        let mut whois = [0; 13];
        server.read_exact(&mut whois).await.unwrap();
        assert_eq!(&whois, b"WHOIS emily\r\n");
        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn raw_requests_resolve_fail_time_out_and_abort() {
        let (mut server, client, mut events, driver) = scripted(
            config()
                .wanted_caps(["labeled-response"])
                .request_timeout(Duration::from_secs(1)),
        );
        let driver = tokio::spawn(driver);
        complete_labelled_registration(&mut server, &mut events).await;

        let whois = tokio::spawn({
            let client = client.clone();
            async move { client.whois("alice").await }
        });
        client_line(&mut server, b"@label=0 WHOIS alice\r\n").await;
        server_line(&mut server, b"@label=0 :ergo.test BATCH +whois-42 labeled-response\r\n@batch=whois-42 :ergo.test 311 papo-driver alice ~alice client.example * :Alice Example\r\n@batch=whois-42 :ergo.test 312 papo-driver alice ergo.test :Ergo Test Network\r\n@batch=whois-42 :ergo.test 317 papo-driver alice 42 1760544000 :seconds idle, signon time\r\n@batch=whois-42 :ergo.test 319 papo-driver alice :@#papo +#rust\r\n@batch=whois-42 :ergo.test 330 papo-driver alice alice :is logged in as\r\n@batch=whois-42 :ergo.test 301 papo-driver alice :away from keyboard\r\n@batch=whois-42 :ergo.test 335 papo-driver alice :is a bot\r\n@batch=whois-42 :ergo.test 671 papo-driver alice :is using a secure connection\r\n@batch=whois-42 :ergo.test 318 papo-driver alice :End of /WHOIS list\r\n:ergo.test BATCH -whois-42\r\n").await;
        let whois = whois.await.unwrap().unwrap();
        assert_eq!(
            (whois.nick.as_str(), whois.user.as_deref()),
            ("alice", Some("~alice"))
        );
        assert_eq!(
            (whois.host.as_deref(), whois.realname.as_deref()),
            (Some("client.example"), Some("Alice Example"))
        );
        assert_eq!(
            (whois.account.as_deref(), whois.server.as_deref()),
            (Some("alice"), Some("ergo.test"))
        );
        assert_eq!(whois.channels, ["@#papo", "+#rust"]);
        assert_eq!(whois.away.as_deref(), Some("away from keyboard"));
        assert!(whois.is_bot && whois.secure);
        assert_eq!(whois.idle, Some(Duration::from_secs(42)));

        server_line(
            &mut server,
            b":ergo.test 005 papo-driver WHOX PREFIX=(ov)@+ :are supported\r\n",
        )
        .await;
        let who = tokio::spawn({
            let client = client.clone();
            async move { client.who("#papo").await }
        });
        client_line(&mut server, b"@label=1 WHO #papo %tcuhnfar\r\n").await;
        server_line(&mut server, b"@label=1 :ergo.test BATCH +who-42 labeled-response\r\n@batch=who-42 :ergo.test 354 papo-driver 0 #papo ~alice client.example alice H@B alice :Alice Example\r\n@batch=who-42 :ergo.test 354 papo-driver 0 #papo ~dan user.example dan G+* 0 :Dan Away\r\n@batch=who-42 :ergo.test 315 papo-driver #papo :End of WHO list\r\n:ergo.test BATCH -who-42\r\n").await;
        let who = who.await.unwrap().unwrap();
        assert_eq!(who.len(), 2);
        assert_eq!(
            (who[0].nick.as_str(), who[0].account.as_deref()),
            ("alice", Some("alice"))
        );
        assert_eq!(
            (who[0].away, who[0].is_bot, who[0].prefixes.as_slice()),
            (false, true, ['@'].as_slice())
        );
        assert_eq!(
            (who[1].away, who[1].is_oper, who[1].prefixes.as_slice()),
            (true, true, ['+'].as_slice())
        );

        assert_join_waits_for_sync(&mut server, &client).await;
        assert_channel_requests(&mut server, &client).await;

        let request = tokio::spawn({
            let client = client.clone();
            async move { client.whois("missing-nick").await }
        });
        client_line(&mut server, b"@label=8 WHOIS missing-nick\r\n").await;
        server_line(
            &mut server,
            b"@label=8 :ergo.test 401 papo-driver missing-nick :No such nick\r\n",
        )
        .await;
        assert!(matches!(
            request.await.unwrap(),
            Err(Error::Server(papo_core::ServerError::Numeric { numeric, .. })) if numeric.code() == 401
        ));

        let timed_out = tokio::spawn({
            let client = client.clone();
            async move { client.raw(Message::new("WHOIS", ["alice"])).await }
        });
        client_line(&mut server, b"@label=9 WHOIS alice\r\n").await;
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(matches!(timed_out.await.unwrap(), Err(Error::Timeout)));

        let aborted =
            tokio::spawn(async move { client.raw(Message::new("WHOIS", ["emily"])).await });
        client_line(&mut server, b"@label=A WHOIS emily\r\n").await;
        server.shutdown().await.unwrap();
        assert!(matches!(aborted.await.unwrap(), Err(Error::Disconnected)));
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn typed_requests_match_unlabelled_responses() {
        let (mut server, client, mut events, driver) = scripted(config());
        let driver = tokio::spawn(driver);
        complete_registration(&mut server, &mut events).await;

        let whois = tokio::spawn({
            let client = client.clone();
            async move { client.whois("alice").await }
        });
        client_line(&mut server, b"WHOIS alice\r\n").await;
        server_line(&mut server, b":ergo.test 311 papo-driver alice ~alice host * :Alice Example\r\n:ergo.test 318 papo-driver alice :End of /WHOIS list\r\n").await;
        assert_eq!(whois.await.unwrap().unwrap().nick, "alice");

        let who = tokio::spawn({
            let client = client.clone();
            async move { client.who("#papo").await }
        });
        client_line(&mut server, b"WHO #papo\r\n").await;
        server_line(&mut server, b":ergo.test 352 papo-driver #papo ~alice host ergo.test alice H@ :0 Alice Example\r\n:ergo.test 315 papo-driver #papo :End of WHO list\r\n").await;
        assert_eq!(who.await.unwrap().unwrap()[0].nick, "alice");

        let join = tokio::spawn({
            let client = client.clone();
            async move { client.join("#papo").await }
        });
        client_line(&mut server, b"JOIN #papo\r\n").await;
        server_line(&mut server, b":papo-driver!papo@localhost JOIN #papo * :Papo Client\r\n:ergo.test 353 papo-driver = #papo :@papo-driver alice\r\n:ergo.test 366 papo-driver #papo :End of /NAMES list\r\n").await;
        assert_eq!(join.await.unwrap().unwrap().members_count, 2);
        client_line(&mut server, b"WHO #papo\r\n").await;

        let list = tokio::spawn({
            let client = client.clone();
            async move { client.list(crate::ListFilter::default()).await }
        });
        client_line(&mut server, b"LIST\r\n").await;
        server_line(&mut server, b":ergo.test 322 papo-driver #papo 42 :Ergonomic IRC\r\n:ergo.test 323 papo-driver :End of /LIST\r\n").await;
        assert_eq!(list.await.unwrap().unwrap()[0].members_count, 42);

        let mode = tokio::spawn({
            let client = client.clone();
            async move { client.mode("#papo", "+t", [] as [&str; 0]).await }
        });
        client_line(&mut server, b"MODE #papo +t\r\n").await;
        server_line(
            &mut server,
            b":papo-driver!papo@localhost MODE #papo +t\r\n",
        )
        .await;
        mode.await.unwrap().unwrap();

        let topic = tokio::spawn({
            let client = client.clone();
            async move { client.topic("#papo", None).await }
        });
        client_line(&mut server, b"TOPIC #papo\r\n").await;
        server_line(
            &mut server,
            b":ergo.test 332 papo-driver #papo :Ergonomic IRC\r\n",
        )
        .await;
        assert_eq!(
            topic.await.unwrap().unwrap().as_deref(),
            Some("Ergonomic IRC")
        );

        let part = tokio::spawn({
            let client = client.clone();
            async move { client.part("#papo", None).await }
        });
        client_line(&mut server, b"PART #papo\r\n").await;
        server_line(
            &mut server,
            b":papo-driver!papo@localhost PART #papo :Leaving\r\n",
        )
        .await;
        part.await.unwrap().unwrap();

        let denied = tokio::spawn({
            let client = client.clone();
            async move { client.join("#invite-only").await }
        });
        client_line(&mut server, b"JOIN #invite-only\r\n").await;
        server_line(
            &mut server,
            b":ergo.test 473 papo-driver #invite-only :Cannot join channel (+i)\r\n",
        )
        .await;
        assert!(matches!(denied.await.unwrap(), Err(Error::Server(_))));

        assert!(matches!(
            client
                .raw(Message::new("VERSION", Vec::<String>::new()))
                .await,
            Err(Error::Unsupported("labeled-response"))
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(1), server.read_u8())
                .await
                .is_err()
        );

        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    enum ReplyKind {
        Reply,
        #[cfg(feature = "bot")]
        PrivateNotice,
    }

    async fn assert_reply_wire(
        message_tags: bool,
        isupport: &str,
        incoming: &[u8],
        expected: &[u8],
        kind: ReplyKind,
    ) {
        let wanted = message_tags.then_some("message-tags");
        let (mut server, _client, mut events, driver) = scripted(config().wanted_caps(wanted));
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        if message_tags {
            server_line(&mut server, b":ergo.test CAP * LS :message-tags\r\n").await;
            client_line(&mut server, b"CAP REQ :message-tags\r\n").await;
            server_line(&mut server, b":ergo.test CAP * ACK :message-tags\r\n").await;
        } else {
            server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
        }
        client_line(&mut server, b"CAP END\r\n").await;
        if !isupport.is_empty() {
            server_line(&mut server, isupport.as_bytes()).await;
        }
        server_line(
            &mut server,
            b":ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :No MOTD\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
        server_line(&mut server, incoming).await;
        let Some(Event::Message(message)) = events.next().await else {
            panic!("expected message event")
        };
        match kind {
            ReplyKind::Reply => {
                message.reply("pong").await.unwrap();
            }
            #[cfg(feature = "bot")]
            ReplyKind::PrivateNotice => {
                message.private_notice("denied").await.unwrap();
            }
        }
        client_line(&mut server, expected).await;
        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn message_reply_obeys_tags_and_clienttagdeny() {
        let tagged = b"@msgid=RfcBTzyxY120003 :emily!e@localhost PRIVMSG #papo :!ping\r\n";
        assert_reply_wire(
            true,
            "",
            tagged,
            b"@+reply=RfcBTzyxY120003 PRIVMSG #papo pong\r\n",
            ReplyKind::Reply,
        )
        .await;
        assert_reply_wire(
            true,
            "",
            b":emily!e@localhost PRIVMSG #papo :!ping\r\n",
            b"PRIVMSG #papo pong\r\n",
            ReplyKind::Reply,
        )
        .await;
        assert_reply_wire(
            false,
            "",
            tagged,
            b"PRIVMSG #papo pong\r\n",
            ReplyKind::Reply,
        )
        .await;
        assert_reply_wire(
            true,
            ":ergo.test 005 papo-driver CLIENTTAGDENY=* :are supported\r\n",
            tagged,
            b"PRIVMSG #papo pong\r\n",
            ReplyKind::Reply,
        )
        .await;
        assert_reply_wire(
            true,
            ":ergo.test 005 papo-driver CLIENTTAGDENY=*,-reply :are supported\r\n",
            tagged,
            b"@+reply=RfcBTzyxY120003 PRIVMSG #papo pong\r\n",
            ReplyKind::Reply,
        )
        .await;
        assert_reply_wire(
            true,
            ":ergo.test 005 papo-driver CLIENTTAGDENY=*,-draft/reply :are supported\r\n",
            tagged,
            b"@+draft/reply=RfcBTzyxY120003 PRIVMSG #papo pong\r\n",
            ReplyKind::Reply,
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn chat_text_uses_plain_splitting_and_rejects_empty_input() {
        let (mut server, client, _events, driver) = scripted(config());
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        assert_eq!(
            client.privmsg("#papo", &"a".repeat(600)).await.unwrap(),
            super::Sent::default()
        );
        let lines = format!(
            "PRIVMSG #papo {}\r\nPRIVMSG #papo {}\r\n",
            "a".repeat(413),
            "a".repeat(187)
        );
        client_line(&mut server, lines.as_bytes()).await;
        client
            .notice("papo-helper", "maintenance soon")
            .await
            .unwrap();
        client_line(&mut server, b"NOTICE papo-helper :maintenance soon\r\n").await;
        assert!(matches!(
            client.privmsg("#papo", "\n").await,
            Err(Error::EmptyText)
        ));
        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn chat_text_uses_multiline_when_negotiated() {
        let config = config().wanted_caps(["batch", "draft/multiline"]);
        let (mut server, client, _events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test CAP * LS :batch draft/multiline=max-bytes=4096,max-lines=100\r\n",
        )
        .await;
        client_line(&mut server, b"CAP REQ :batch draft/multiline\r\n").await;
        server_line(
            &mut server,
            b":ergo.test CAP * ACK :batch draft/multiline\r\n",
        )
        .await;
        client_line(&mut server, b"CAP END\r\n").await;
        client.privmsg("#papo", &"a".repeat(600)).await.unwrap();
        let batch = format!(
            "BATCH +b0-0 draft/multiline #papo\r\n@batch=b0-0 PRIVMSG #papo {}\r\n@batch=b0-0;draft/multiline-concat PRIVMSG #papo {}\r\nBATCH -b0-0\r\n",
            "a".repeat(413),
            "a".repeat(187)
        );
        client_line(&mut server, batch.as_bytes()).await;
        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn chat_text_returns_metadata_from_a_labelled_echo() {
        let config = config().wanted_caps(["echo-message", "labeled-response"]);
        let (mut server, client, _events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test CAP * LS :echo-message labeled-response\r\n",
        )
        .await;
        client_line(&mut server, b"CAP REQ :echo-message labeled-response\r\n").await;
        server_line(
            &mut server,
            b":ergo.test CAP * ACK :echo-message labeled-response\r\n",
        )
        .await;
        client_line(&mut server, b"CAP END\r\n").await;

        let sender = client.clone();
        let sent = tokio::spawn(async move { sender.privmsg("#papo", "hello everyone").await });
        client_line(&mut server, b"@label=0 PRIVMSG #papo :hello everyone\r\n").await;
        server_line(
            &mut server,
            b"@label=0;msgid=RfcBTzyxY120004;time=2026-02-03T04:05:06.789Z :papo-driver!papo@localhost PRIVMSG #papo :hello everyone\r\n",
        )
        .await;
        let sent = sent.await.unwrap().unwrap();
        assert_eq!(sent.msgid.as_deref(), Some("RfcBTzyxY120004"));
        assert_eq!(
            sent.time,
            Some(UNIX_EPOCH + Duration::new(1_770_091_506, 789_000_000))
        );

        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    async fn assert_reaction_wire(isupport: &str, remove: bool, expected: Option<&[u8]>) {
        let (mut server, client, mut events, driver) =
            scripted(config().wanted_caps(["message-tags"]));
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :message-tags\r\n").await;
        client_line(&mut server, b"CAP REQ :message-tags\r\n").await;
        server_line(&mut server, b":ergo.test CAP * ACK :message-tags\r\n").await;
        client_line(&mut server, b"CAP END\r\n").await;
        if !isupport.is_empty() {
            server_line(&mut server, isupport.as_bytes()).await;
        }
        server_line(
            &mut server,
            b":ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :No MOTD\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
        server_line(
            &mut server,
            b"@msgid=RfcBTzyxY120004 :emily!e@localhost PRIVMSG #papo :ship it\r\n",
        )
        .await;
        let message = next_message(&mut events).await;
        let result = if remove {
            client.unreact(&message, "👍").await
        } else {
            client.react(&message, "👍").await
        };
        if let Some(expected) = expected {
            result.unwrap();
            client_line(&mut server, expected).await;
        } else {
            assert!(matches!(result, Err(Error::Unsupported("reaction tag"))));
            assert!(
                tokio::time::timeout(Duration::from_millis(1), server.read_u8())
                    .await
                    .is_err()
            );
        }
        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reactions_obey_clienttagdeny() {
        assert_reaction_wire(
            "",
            false,
            Some(b"@+reply=RfcBTzyxY120004;+draft/react=\xF0\x9F\x91\x8D TAGMSG #papo\r\n"),
        )
        .await;
        assert_reaction_wire(
            "",
            true,
            Some(b"@+reply=RfcBTzyxY120004;+draft/unreact=\xF0\x9F\x91\x8D TAGMSG #papo\r\n"),
        )
        .await;
        assert_reaction_wire(
            ":ergo.test 005 papo-driver CLIENTTAGDENY=draft/react :are supported\r\n",
            false,
            None,
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn typing_throttles_active_per_target_and_sends_terminal_states() {
        let (mut server, client, _events, driver) =
            scripted(config().wanted_caps(["message-tags"]));
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :message-tags\r\n").await;
        client_line(&mut server, b"CAP REQ :message-tags\r\n").await;
        server_line(&mut server, b":ergo.test CAP * ACK :message-tags\r\n").await;
        client_line(&mut server, b"CAP END\r\n").await;

        client.typing("#papo", Typing::Active).await.unwrap();
        client_line(&mut server, b"@+typing=active TAGMSG #papo\r\n").await;
        client.typing("#PAPO", Typing::Active).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(1), server.read_u8())
                .await
                .is_err()
        );
        tokio::time::advance(Duration::from_secs(2)).await;
        client.typing("#papo", Typing::Active).await.unwrap();
        client.typing("#papo", Typing::Paused).await.unwrap();
        client.typing("#papo", Typing::Done).await.unwrap();
        client_line(
            &mut server,
            b"@+typing=paused TAGMSG #papo\r\n@+typing=done TAGMSG #papo\r\n",
        )
        .await;
        tokio::time::advance(Duration::from_secs(1)).await;
        client.typing("#papo", Typing::Active).await.unwrap();
        client_line(&mut server, b"@+typing=active TAGMSG #papo\r\n").await;

        server_line(
            &mut server,
            b":ergo.test 005 papo-driver CLIENTTAGDENY=typing :are supported\r\n",
        )
        .await;
        while !client.isupport().clienttagdeny().denies("typing") {
            tokio::task::yield_now().await;
        }
        client.typing("emily", Typing::Done).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(1), server.read_u8())
                .await
                .is_err()
        );

        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn redaction_requires_its_capability_and_propagates_fail() {
        let caps = ["draft/message-redaction", "labeled-response"];
        let (mut server, client, _events, driver) = scripted(config().wanted_caps(caps));
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test CAP * LS :draft/message-redaction labeled-response\r\n",
        )
        .await;
        client_line(
            &mut server,
            b"CAP REQ :draft/message-redaction labeled-response\r\n",
        )
        .await;
        server_line(
            &mut server,
            b":ergo.test CAP * ACK :draft/message-redaction labeled-response\r\n",
        )
        .await;
        client_line(&mut server, b"CAP END\r\n").await;

        let sender = client.clone();
        let redacted = tokio::spawn(async move {
            sender
                .redact("#papo", "RfcBTzyxY120004", Some("duplicate message"))
                .await
        });
        client_line(
            &mut server,
            b"@label=0 REDACT #papo RfcBTzyxY120004 :duplicate message\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=0 :papo-driver!papo@localhost REDACT #papo RfcBTzyxY120004 :duplicate message\r\n",
        )
        .await;
        assert!(redacted.await.unwrap().is_ok());

        let sender = client.clone();
        let rejected =
            tokio::spawn(async move { sender.redact("#papo", "RfcBTzyxY120005", None).await });
        client_line(&mut server, b"@label=1 REDACT #papo RfcBTzyxY120005\r\n").await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test FAIL REDACT FORBIDDEN #papo :Insufficient privileges\r\n",
        )
        .await;
        assert!(matches!(
            rejected.await.unwrap(),
            Err(Error::Server(papo_core::ServerError::Standard { command, code, .. }))
                if command == "REDACT" && code == "FORBIDDEN"
        ));

        server_line(
            &mut server,
            b":ergo.test CAP * DEL :draft/message-redaction\r\n",
        )
        .await;
        while client.has_cap("draft/message-redaction") {
            tokio::task::yield_now().await;
        }
        assert!(matches!(
            client.redact("#papo", "RfcBTzyxY120006", None).await,
            Err(Error::Unsupported("draft/message-redaction"))
        ));

        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn labels_distinguish_our_echo_from_another_session() {
        let caps = ["echo-message", "labeled-response"];
        let (mut server, client, mut events, driver) = scripted(config().wanted_caps(caps));
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test CAP * LS :echo-message labeled-response\r\n",
        )
        .await;
        client_line(&mut server, b"CAP REQ :echo-message labeled-response\r\n").await;
        server_line(
            &mut server,
            b":ergo.test CAP * ACK :echo-message labeled-response\r\n",
        )
        .await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(
            &mut server,
            b":ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :No MOTD\r\n",
        )
        .await;
        while !matches!(events.next().await, Some(Event::Registered(_))) {}

        let sender = client.clone();
        let sent = tokio::spawn(async move { sender.privmsg("#papo", "from this device").await });
        client_line(&mut server, b"@label=0 PRIVMSG #papo :from this device\r\n").await;
        server_line(
            &mut server,
            b"@label=0;msgid=RfcBTzyxY120004 :papo-driver!papo@localhost PRIVMSG #papo :from this device\r\n",
        )
        .await;
        assert!(sent.await.unwrap().is_ok());
        let echo = next_message(&mut events).await;
        assert_eq!(echo.origin(), Origin::Echo);
        assert!(!echo.is_live());

        server_line(
            &mut server,
            b"@msgid=RfcBTzyxY120005 :papo-driver!papo@localhost PRIVMSG #papo :from another device\r\n",
        )
        .await;
        let other = next_message(&mut events).await;
        assert_eq!(other.origin(), Origin::OtherSession);
        assert!(!other.is_live());

        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn private_reply_keeps_channel_context() {
        assert_reply_wire(
            true,
            "",
            b"@msgid=RfcBTzyxY120003;+channel-context=#papo :emily!e@localhost PRIVMSG papo-driver :!ping\r\n",
            b"@+reply=RfcBTzyxY120003;+channel-context=#papo PRIVMSG emily pong\r\n",
            ReplyKind::Reply,
        )
        .await;
    }

    #[cfg(feature = "bot")]
    #[tokio::test(flavor = "current_thread")]
    async fn private_notice_adds_the_source_channel_context() {
        assert_reply_wire(
            true,
            "",
            b"@msgid=RfcBTzyxY120003 :emily!e@localhost PRIVMSG #papo :!op alice\r\n",
            b"@+reply=RfcBTzyxY120003;+channel-context=#papo NOTICE emily denied\r\n",
            ReplyKind::PrivateNotice,
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_send_rejects_an_invalid_message() {
        let (mut server, client, _events, driver) = scripted(config());
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        let error = client
            .send(papo_proto::Message::new("WHO IS", ["emily"]))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            Error::Build(papo_proto::BuildError::InvalidCommand)
        ));
        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn quit_resolves_after_server_eof() {
        let (mut server, client, mut events, driver) = scripted(config());
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        let quit = tokio::spawn(async move { client.quit(Some("leaving for maintenance")).await });
        let mut line = [0; 31];
        server.read_exact(&mut line).await.unwrap();
        assert_eq!(&line, b"QUIT :leaving for maintenance\r\n");
        server.shutdown().await.unwrap();
        assert!(quit.await.unwrap().is_ok());
        assert!(driver.await.unwrap().is_ok());
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected { .. })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn quit_resolves_after_io_failure_and_reports_its_cause() {
        let (client_stream, mut server) = tokio::io::duplex(4096);
        let read_error = Arc::new(AtomicBool::new(false));
        let stream = FailingStream {
            inner: client_stream,
            read_error: Arc::clone(&read_error),
        };
        let (client, mut events, driver) = Driver::with_stream(stream, Session::new(config()));
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        let quit = tokio::spawn(async move { client.quit(None).await });
        client_line(&mut server, b"QUIT\r\n").await;

        read_error.store(true, Ordering::Relaxed);
        server
            .write_all(b":ergo.test NOTICE papo-driver :connection closing\r\n")
            .await
            .unwrap();

        assert!(quit.await.unwrap().is_ok());
        assert!(driver.await.unwrap().is_ok());
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                reason,
                will_retry: false,
            }) if reason == "original read failure"
        ));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn quit_times_out_after_ten_seconds() {
        let (mut server, client, mut events, driver) = scripted(config());
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        let quit = tokio::spawn(async move { client.quit(None).await });
        let mut line = [0; 6];
        server.read_exact(&mut line).await.unwrap();
        assert_eq!(&line, b"QUIT\r\n");
        tokio::time::advance(Duration::from_secs(9)).await;
        assert!(!quit.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(matches!(quit.await.unwrap(), Err(Error::Timeout)));
        assert!(driver.await.unwrap().is_ok());
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected {
                reason,
                will_retry: false,
            }) if reason == "QUIT was not acknowledged within 10 seconds"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_every_client_sends_quit() {
        let (mut server, client, _events, driver) = scripted(config());
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        drop(client);
        let mut line = [0; 6];
        server.read_exact(&mut line).await.unwrap();
        assert_eq!(&line, b"QUIT\r\n");
        server.shutdown().await.unwrap();
        assert!(driver.await.unwrap().is_ok());
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn full_event_channel_still_serves_quit_and_its_timer() {
        let (mut server, client, events, driver) = scripted_with_capacity(config(), 1);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server
            .write_all(b":ergo.test NOTICE papo-driver :maintenance soon\r\n:ergo.test NOTICE papo-driver :still queued\r\n")
            .await
            .unwrap();
        while events.receiver.len() != 1 {
            tokio::task::yield_now().await;
        }

        let quit = tokio::spawn(async move { client.quit(None).await });
        let mut line = [0; 6];
        server.read_exact(&mut line).await.unwrap();
        assert_eq!(&line, b"QUIT\r\n");
        tokio::time::advance(Duration::from_secs(10)).await;

        assert!(matches!(quit.await.unwrap(), Err(Error::Timeout)));
        assert!(driver.await.unwrap().is_ok());
        assert_eq!(events.receiver.len(), 1);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn session_deadline_sends_keepalive_ping() {
        let config = config()
            .ping_interval(Duration::from_secs(30))
            .ping_timeout(Duration::from_secs(120));
        let (mut server, _client, _events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        tokio::time::advance(Duration::from_secs(30)).await;
        let mut ping = [0; 14];
        server.read_exact(&mut ping).await.unwrap();
        assert_eq!(&ping, b"PING :papo-0\r\n");
        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_reads_owned_state_snapshots() {
        let (mut server, client, mut events, driver) = scripted(config());
        assert!(!client.snapshot().connected());
        let driver = tokio::spawn(driver);
        complete_registration(&mut server, &mut events).await;
        assert!(client.snapshot().connected());

        server_line(
            &mut server,
            b":papo-driver!papo@localhost JOIN #Papo\r\n:ergo.test 353 papo-driver = #Papo :@papo-driver +emily\r\n:ergo.test 366 papo-driver #Papo :End of NAMES list\r\n",
        )
        .await;
        while !matches!(events.next().await, Some(Event::ChannelSynced { .. })) {}
        client.state(|state| {
            let channel = state.channel("#papo").unwrap();
            assert!(channel.is_synced());
            assert_eq!(channel.member("EMILY").unwrap().prefixes(), &['+']);
        });

        let unchanged = client.snapshot();
        server_line(
            &mut server,
            b":ergo.test NOTICE papo-driver :maintenance soon\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::Message(_))));
        assert!(Arc::ptr_eq(&unchanged, &client.snapshot()));

        server_line(
            &mut server,
            b":papo-driver!papo@localhost PART #Papo :leaving\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::Parted { .. })));
        assert!(!Arc::ptr_eq(&unchanged, &client.snapshot()));
        assert!(client.state(|state| state.channel("#papo").is_none()));

        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
        assert!(!client.snapshot().connected());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn public_state_accessors_report_the_registered_session() {
        let config = config()
            .wanted_caps(["extended-monitor"])
            .sasl(Sasl::plain("alice", "hunter2"));
        let (mut server, client, mut events, driver) = scripted(config);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test CAP * LS :extended-monitor sasl=PLAIN\r\n",
        )
        .await;
        client_line(&mut server, b"CAP REQ :extended-monitor sasl\r\n").await;
        server_line(
            &mut server,
            b":ergo.test CAP papo-driver ACK :extended-monitor sasl\r\n",
        )
        .await;
        client_line(&mut server, b"AUTHENTICATE PLAIN\r\n").await;
        server_line(&mut server, b"AUTHENTICATE +\r\n").await;
        client_line(&mut server, b"AUTHENTICATE AGFsaWNlAGh1bnRlcjI=\r\n").await;
        server_line(&mut server, b":ergo.test 900 papo-driver papo-driver!papo@localhost emily :You are now logged in as emily\r\n:ergo.test 903 papo-driver :SASL authentication successful\r\n").await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(&mut server, b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
        assert!(!client.snapshot().away());

        server_line(&mut server, b":ergo.test 005 papo-driver CASEMAPPING=ascii CHANTYPES=# PREFIX=(ov)@+ MONITOR=100 NICKLEN=27 :are supported\r\n:ergo.test 306 papo-driver :You have been marked as being away\r\n:papo-driver!papo@localhost MODE papo-driver +iw\r\n:papo-driver!papo@localhost JOIN #Papo\r\n:ergo.test 353 papo-driver = #Papo :@papo-driver alice!~alice@cloak.example\r\n:ergo.test 366 papo-driver #Papo :End of NAMES list\r\n:ergo.test 730 papo-driver :bob!bob@users.example\r\n").await;
        while !matches!(events.next().await, Some(Event::MonitorOnline { .. })) {}

        assert_eq!(client.nick(), "papo-driver");
        client.state(|state| {
            assert_eq!(state.nick(), "papo-driver");
            assert_eq!(state.account(), Some("emily"));
            assert!(state.away());
            let mut modes: Vec<_> = state.user_modes().collect();
            modes.sort_unstable();
            assert_eq!(modes, ['i', 'w']);
            assert_eq!(
                state
                    .channels()
                    .map(papo_core::Channel::name)
                    .collect::<Vec<_>>(),
                ["#Papo"]
            );
            assert_eq!(state.user("ALICE").unwrap().user(), Some("~alice"));
            assert_eq!(state.monitor_online().next().unwrap().nick(), "bob");
            assert_eq!(state.isupport().nicklen(), Some(27));
        });
        let snapshot = client.snapshot();
        assert_eq!(snapshot.nick(), "papo-driver");
        assert_eq!(snapshot.account(), Some("emily"));

        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    #[allow(clippy::too_many_lines)]
    async fn every_state_change_replaces_the_snapshot() {
        let config = config().wanted_caps(["extended-monitor"]);
        let (mut server, client, mut events, driver) = scripted(config);
        let driver = tokio::spawn(driver);
        complete_extended_monitor_registration(&mut server, &mut events).await;
        let mut previous = client.snapshot();

        server_line(&mut server, b":papo-driver!papo@localhost JOIN #Papo\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Joined { .. })));
        previous = changed_snapshot(&client, &previous, "JOIN");
        assert!(previous.channel("#papo").is_some());

        server_line(&mut server, b":ergo.test MODE papo-driver +i\r\n").await;
        assert!(matches!(
            events.next().await,
            Some(Event::ModeChanged { .. })
        ));
        previous = changed_snapshot(&client, &previous, "MODE");
        assert_eq!(previous.user_modes().collect::<Vec<_>>(), ['i']);

        server_line(
            &mut server,
            b":emily!em@staff.example TOPIC #Papo :Release planning\r\n",
        )
        .await;
        assert!(matches!(
            events.next().await,
            Some(Event::TopicChanged { .. })
        ));
        previous = changed_snapshot(&client, &previous, "TOPIC");
        assert_eq!(
            previous.channel("#papo").unwrap().topic(),
            Some("Release planning")
        );

        server_line(
            &mut server,
            b":papo-driver!papo@localhost ACCOUNT emily\r\n",
        )
        .await;
        assert!(matches!(
            events.next().await,
            Some(Event::AccountChanged { .. })
        ));
        previous = changed_snapshot(&client, &previous, "ACCOUNT");
        assert_eq!(
            previous.user("papo-driver").unwrap().account(),
            &papo_core::Account::LoggedIn("emily".into())
        );

        server_line(
            &mut server,
            b":papo-driver!papo@localhost AWAY :Gone to lunch\r\n",
        )
        .await;
        assert!(matches!(
            events.next().await,
            Some(Event::AwayChanged { .. })
        ));
        previous = changed_snapshot(&client, &previous, "AWAY");
        assert_eq!(
            previous.user("papo-driver").unwrap().away(),
            Some("Gone to lunch")
        );

        server_line(
            &mut server,
            b":papo-driver!papo@localhost NICK papo-reader\r\n",
        )
        .await;
        assert!(matches!(
            events.next().await,
            Some(Event::NickChanged { .. })
        ));
        previous = changed_snapshot(&client, &previous, "NICK");
        assert_eq!(previous.nick(), "papo-reader");

        server_line(
            &mut server,
            b":ergo.test 730 papo-reader :alice!~alice@cloak.example\r\n",
        )
        .await;
        assert!(matches!(
            events.next().await,
            Some(Event::MonitorOnline { .. })
        ));
        previous = changed_snapshot(&client, &previous, "MONITOR online");
        assert_eq!(previous.monitor_online().count(), 1);

        server_line(&mut server, b":ergo.test 731 papo-reader :alice\r\n").await;
        assert!(matches!(
            events.next().await,
            Some(Event::MonitorOffline { .. })
        ));
        previous = changed_snapshot(&client, &previous, "MONITOR offline");
        assert_eq!(previous.monitor_online().count(), 0);

        server_line(
            &mut server,
            b":ergo.test 005 papo-reader NICKLEN=31 :are supported\r\nPING :snapshot\r\n",
        )
        .await;
        client_line(&mut server, b"PONG :snapshot\r\n").await;
        previous = changed_snapshot(&client, &previous, "005");
        assert_eq!(previous.isupport().nicklen(), Some(31));

        server_line(
            &mut server,
            b":papo-reader!papo@localhost PART #Papo :Leaving\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::Parted { .. })));
        let current = changed_snapshot(&client, &previous, "PART");
        assert!(current.channel("#papo").is_none());

        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_exposes_capability_aliases_and_casemapping() {
        let config = config().wanted_caps(["draft/no-implicit-names"]);
        let (mut server, client, mut events, driver) = scripted(config);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :no-implicit-names\r\n").await;
        client_line(&mut server, b"CAP REQ :no-implicit-names\r\n").await;
        server_line(
            &mut server,
            b":ergo.test CAP papo-driver ACK :no-implicit-names\r\n",
        )
        .await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(
            &mut server,
            b":ergo.test 005 papo-driver CASEMAPPING=ascii NICKLEN=27 :are supported\r\n:ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));

        assert!(client.has_cap("no-implicit-names"));
        assert!(client.has_cap("draft/no-implicit-names"));
        assert!(!client.has_cap("message-tags"));
        assert_eq!(client.isupport().nicklen(), Some(27));
        assert_eq!(client.casemap("#PAPO[dev]"), "#papo[dev]");
        assert_ne!(client.casemap("#PAPO[dev]"), client.casemap("#papo{dev}"));
        client.state(|state| assert!(state.has_cap("draft/no-implicit-names")));

        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn monitor_offline_snapshot_keeps_other_online_users() {
        let config = config().wanted_caps(["extended-monitor"]);
        let (mut server, client, mut events, driver) = scripted(config);
        let driver = tokio::spawn(driver);
        complete_extended_monitor_registration(&mut server, &mut events).await;
        server_line(
            &mut server,
            b":ergo.test 730 papo-driver :alice!~alice@cloak.example,bob!bob@users.example\r\n",
        )
        .await;
        assert!(matches!(
            events.next().await,
            Some(Event::MonitorOnline { .. })
        ));
        assert!(matches!(
            events.next().await,
            Some(Event::MonitorOnline { .. })
        ));

        server_line(&mut server, b":ergo.test 731 papo-driver :alice\r\n").await;
        assert!(
            matches!(events.next().await, Some(Event::MonitorOffline { nick }) if nick == "alice")
        );
        let online: Vec<_> = client.state(|state| {
            state
                .monitor_online()
                .map(|user| user.nick().to_owned())
                .collect()
        });
        assert_eq!(online, ["bob"]);

        server.shutdown().await.unwrap();
        _ = driver.await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disconnect_clears_monitor_presence_snapshot() {
        let config = config().wanted_caps(["extended-monitor"]);
        let (mut server, client, mut events, driver) = scripted(config);
        let driver = tokio::spawn(driver);
        complete_extended_monitor_registration(&mut server, &mut events).await;
        server_line(
            &mut server,
            b":ergo.test 730 papo-driver :alice!~alice@cloak.example\r\n",
        )
        .await;
        assert!(matches!(
            events.next().await,
            Some(Event::MonitorOnline { .. })
        ));
        assert_eq!(client.state(|state| state.monitor_online().count()), 1);

        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
        assert_eq!(client.state(|state| state.monitor_online().count()), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn registers_accounts_with_typed_results() {
        let settings = config().wanted_caps(["draft/account-registration", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(&mut server, b":ergo.test CAP * LS :draft/account-registration=custom-account-name labeled-response\r\n").await;
        client_line(
            &mut server,
            b"CAP REQ :draft/account-registration labeled-response\r\n",
        )
        .await;
        server_line(
            &mut server,
            b":ergo.test CAP * ACK :draft/account-registration labeled-response\r\n",
        )
        .await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(&mut server, b":ergo.test 001 papo-driver :Welcome\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));

        let register = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .register(
                        Some("alice"),
                        Some("alice@example.org"),
                        papo_core::Secret::new("correct horse battery staple"),
                    )
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=0 REGISTER alice alice@example.org :correct horse battery staple\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=0 :ergo.test REGISTER VERIFICATION_REQUIRED alice :Verification code sent\r\n",
        )
        .await;
        assert!(
            matches!(register.await.unwrap().unwrap(), super::AccountRegistration::VerificationRequired { account, .. } if account == "alice")
        );

        let verify = tokio::spawn(async move { client.verify("alice", "expired-code").await });
        client_line(&mut server, b"@label=1 VERIFY alice expired-code\r\n").await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test FAIL VERIFY INVALID_CODE :Invalid verification code\r\n",
        )
        .await;
        assert!(matches!(
            verify.await.unwrap(),
            Err(Error::AccountRegistration(
                papo_core::AccountRegistrationError::InvalidCode
            ))
        ));

        driver.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reads_changes_and_caches_persistence() {
        let settings = config().wanted_caps(["draft/persistence", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test CAP * LS :draft/persistence labeled-response\r\n",
        )
        .await;
        client_line(
            &mut server,
            b"CAP REQ :draft/persistence labeled-response\r\n",
        )
        .await;
        server_line(
            &mut server,
            b":ergo.test CAP * ACK :draft/persistence labeled-response\r\n",
        )
        .await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(&mut server, b":ergo.test 001 papo-driver :Welcome\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));

        let get = tokio::spawn({
            let client = client.clone();
            async move { client.persistence().get().await }
        });
        client_line(&mut server, b"@label=0 PERSISTENCE GET\r\n").await;
        server_line(
            &mut server,
            b"@label=0 :ergo.test PERSISTENCE STATUS OFF OFF\r\n",
        )
        .await;
        assert!(!get.await.unwrap().unwrap().effective);

        let set = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .persistence()
                    .set(papo_core::PersistenceSetting::On)
                    .await
            }
        });
        client_line(&mut server, b"@label=1 PERSISTENCE SET ON\r\n").await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test PERSISTENCE STATUS ON ON\r\n",
        )
        .await;
        assert!(set.await.unwrap().unwrap().effective);
        assert!(client.persistence().state().unwrap().effective);
        driver.abort();
    }
}
