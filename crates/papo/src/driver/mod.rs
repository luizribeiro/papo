mod catchup;
mod connect;

#[cfg(all(test, not(browser)))]
use catchup::extend_catchup;
#[cfg(all(test, feature = "unstable"))]
use catchup::should_fetch_history;
use catchup::{Catchup, CatchupCursor, advance_catchup, begin_catchup};

#[cfg(not(browser))]
use connect::apply_sts_action;
#[cfg(all(test, not(browser)))]
use connect::{is_fatal_failure, record_sts_upgrade};
use connect::{open_connection, run_plan};

use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    task,
    time::{Duration, SystemTime},
};
use web_time::Instant;

use futures_core::Stream;
use papo_core::{
    Connection, HistoryQuery, RequestError, RequestId, Response, ResponseMatcher, Session,
    TextKind, TransmitToken, sts::StsStore,
};
use papo_proto::{Command as ProtoCommand, Line, Numeric, Typing};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

use crate::{
    Error, Event, InvalidInput, Reconnect,
    address::ServerAddress,
    client::{Client, Sent},
    event_impl as event,
    state::State,
    transport::TlsConfig,
};

pub(crate) const DEFAULT_CHANNEL_CAPACITY: usize = 1024;
const QUIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const STABLE_CONNECTION: Duration = Duration::from_secs(60);
pub(super) const SLOW_CONSUMER_REASON: &str = "SlowConsumer";

#[cfg(not(browser))]
pub(crate) trait ConnectionIo: AsyncRead + AsyncWrite + Unpin + Send {}
#[cfg(not(browser))]
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ConnectionIo for T {}
#[cfg(browser)]
pub(crate) trait ConnectionIo: AsyncRead + AsyncWrite + Unpin {}
#[cfg(browser)]
impl<T: AsyncRead + AsyncWrite + Unpin> ConnectionIo for T {}
type BoxConnection = Box<dyn ConnectionIo>;
#[cfg(not(browser))]
type ConnectFuture = Pin<Box<dyn Future<Output = Result<BoxConnection, Error>> + Send>>;
#[cfg(browser)]
type ConnectFuture = Pin<Box<dyn Future<Output = Result<BoxConnection, Error>>>>;
pub(crate) type Connector = Arc<dyn Fn(ConnectionPlan) -> ConnectFuture + Send + Sync>;
type BackoffEntropy = Box<dyn FnMut() -> u64 + Send>;

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
    #[cfg(feature = "unstable")]
    PersistenceGet,
    #[cfg(feature = "unstable")]
    PersistenceSet(papo_core::PersistenceSetting),
    #[cfg(feature = "unstable")]
    BouncerNetworks,
    #[cfg(feature = "unstable")]
    BouncerAdd(papo_core::NetworkConfig),
    #[cfg(feature = "unstable")]
    BouncerChange(papo_core::NetworkId, papo_core::NetworkConfig),
    #[cfg(feature = "unstable")]
    BouncerRemove(papo_core::NetworkId),
    Metadata(papo_core::MetadataCommand),
    Rename {
        old: String,
        new: String,
        reason: String,
    },
    #[cfg(feature = "unstable")]
    Relay {
        channel: String,
        nick: String,
        text: String,
    },
    #[cfg(feature = "unstable")]
    WebPushRegister {
        endpoint: String,
        p256dh: String,
        auth: papo_core::Secret,
    },
    #[cfg(feature = "unstable")]
    WebPushUnregister(String),
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
            #[cfg(feature = "unstable")]
            Self::PersistenceGet => session.persistence_get(now())?,
            #[cfg(feature = "unstable")]
            Self::PersistenceSet(setting) => session.persistence_set(now(), setting)?,
            #[cfg(feature = "unstable")]
            Self::BouncerNetworks => return session.bouncer_networks(now()),
            #[cfg(feature = "unstable")]
            Self::BouncerAdd(config) => return session.bouncer_add(now(), &config),
            #[cfg(feature = "unstable")]
            Self::BouncerChange(id, config) => {
                return session.bouncer_change(now(), &id, &config);
            }
            #[cfg(feature = "unstable")]
            Self::BouncerRemove(id) => return session.bouncer_remove(now(), &id),
            Self::Metadata(command) => return session.metadata(now(), command),
            Self::Rename { old, new, reason } => return session.rename(now(), &old, &new, &reason),
            #[cfg(feature = "unstable")]
            Self::Relay {
                channel,
                nick,
                text,
            } => return session.relay(now(), &channel, &nick, &text),
            #[cfg(feature = "unstable")]
            Self::WebPushRegister {
                endpoint,
                p256dh,
                auth,
            } => {
                let auth = auth.expose().ok_or(RequestError::InvalidSecret)?;
                return session.webpush_register(now(), &endpoint, &p256dh, auth);
            }
            #[cfg(feature = "unstable")]
            Self::WebPushUnregister(endpoint) => {
                return session.webpush_unregister(now(), &endpoint);
            }
        }
        Ok(before)
    }
}

pub(crate) enum Command {
    Send {
        message: Line,
        result: oneshot::Sender<Result<(), Error>>,
    },
    Request {
        message: Line,
        matcher: Option<ResponseMatcher>,
        result: oneshot::Sender<Result<Response, Error>>,
    },
    Join {
        channel: crate::ChannelName,
        key: Option<crate::Secret>,
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
        result: oneshot::Sender<Result<Response, Error>>,
    },
    MonitorAdd {
        nicks: Vec<String>,
        result: oneshot::Sender<Result<(), Error>>,
    },
    MonitorRemove {
        nicks: Vec<String>,
        result: oneshot::Sender<Result<(), Error>>,
    },
    MonitorClear {
        result: oneshot::Sender<Result<(), Error>>,
    },
    SendText {
        kind: TextKind,
        target: String,
        text: String,
        tags: Vec<(String, String)>,
        fallbacks: Vec<crate::Fallback>,
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
    Reconnect {
        result: oneshot::Sender<Result<(), Error>>,
    },
    Quit {
        message: Line,
        result: oneshot::Sender<Result<(), Error>>,
    },
}

enum PendingRequest {
    Response(oneshot::Sender<Result<Response, Error>>),
    Text {
        target: String,
        fallbacks: Vec<crate::Fallback>,
        result: oneshot::Sender<Result<Sent, Error>>,
    },
}

impl PendingRequest {
    fn is_closed(&self) -> bool {
        match self {
            Self::Response(result) => result.is_closed(),
            Self::Text { result, .. } => result.is_closed(),
        }
    }

    fn respond(self, response: Response) {
        match self {
            Self::Response(result) => _ = result.send(Ok(response)),
            Self::Text {
                target,
                fallbacks,
                result,
            } => {
                let response = response.error().map_or_else(
                    || Ok(Sent::from_response(&response, &target, fallbacks)),
                    |error| Err(Error::server(error)),
                );
                _ = result.send(response);
            }
        }
    }

    fn fail(self, error: Error) {
        match self {
            Self::Response(result) => _ = result.send(Err(error)),
            Self::Text { result, .. } => _ = result.send(Err(error)),
        }
    }
}

enum PendingTransmit {
    Empty(oneshot::Sender<Result<(), Error>>),
    Text {
        target: String,
        fallbacks: Vec<crate::Fallback>,
        result: oneshot::Sender<Result<Sent, Error>>,
    },
}

impl PendingTransmit {
    fn complete(self) {
        match self {
            Self::Empty(result) => _ = result.send(Ok(())),
            Self::Text {
                target,
                fallbacks,
                result,
            } => _ = result.send(Ok(Sent::without_echo(&target, fallbacks))),
        }
    }

    fn fail(self, error: Error) {
        match self {
            Self::Empty(result) => _ = result.send(Err(error)),
            Self::Text { result, .. } => _ = result.send(Err(error)),
        }
    }
}

struct Quit {
    deadline: Instant,
    results: Vec<oneshot::Sender<Result<(), Error>>>,
}

struct EventSink {
    regular: mpsc::Sender<Event>,
    disconnected: oneshot::Sender<VecDeque<Event>>,
    connected: Arc<AtomicBool>,
    state: Arc<RwLock<Arc<State>>>,
    commands: mpsc::WeakSender<Command>,
}

impl EventSink {
    fn finish(self, pending: VecDeque<Event>) {
        drop(self.regular);
        _ = self.disconnected.send(pending);
    }
}

pub(crate) struct Channels {
    pub(crate) client: Client,
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
    #[cfg_attr(browser, allow(dead_code))]
    pub(crate) tls: TlsConfig,
}

#[derive(Default)]
pub(crate) struct PostRegistration {
    pub(crate) bot: bool,
    pub(crate) channels: Vec<Autojoin>,
    pub(crate) history_on_reconnect: bool,
    pub(crate) disconnected_at: Option<std::time::SystemTime>,
}

#[derive(Clone, Debug)]
pub(crate) struct Autojoin {
    pub(crate) channel: String,
    pub(crate) key: Option<papo_core::Secret>,
    catchup_cursor: Option<CatchupCursor>,
}

impl Autojoin {
    pub(crate) fn new(channel: impl Into<String>, key: Option<papo_core::Secret>) -> Self {
        Self {
            channel: channel.into(),
            key,
            catchup_cursor: None,
        }
    }
}

impl From<&str> for Autojoin {
    fn from(channel: &str) -> Self {
        Self::new(channel, None)
    }
}

/// The events produced by a running IRC client.
///
/// Delivery is bounded. If events are not consumed, socket reads stall while
/// client commands and session timers continue to be served. If the bounded
/// overflow also fills, the connection ends with
/// [`DisconnectCause::SlowConsumer`](crate::DisconnectCause::SlowConsumer) and follows the configured reconnect
/// policy. Move slow work out of the event loop or raise
/// [`Builder::event_capacity`](crate::Builder::event_capacity) to avoid this.
pub struct Events {
    receiver: mpsc::Receiver<Event>,
    disconnected: Option<oneshot::Receiver<VecDeque<Event>>>,
    pending: VecDeque<Event>,
}

impl Events {
    /// Waits for the next event, or returns `None` after the driver stops.
    pub async fn next(&mut self) -> Option<Event> {
        let next = std::future::poll_fn(|context| Pin::new(&mut *self).poll_next(context));
        #[cfg(all(test, not(browser)))]
        let event = test_support::within(next).await;
        #[cfg(any(not(test), browser))]
        let event = next.await;
        event
    }
}

impl Stream for Events {
    type Item = Event;

    fn poll_next(
        mut self: Pin<&mut Self>,
        context: &mut task::Context<'_>,
    ) -> task::Poll<Option<Self::Item>> {
        if let Some(event) = self.pending.pop_front() {
            return task::Poll::Ready(Some(event));
        }
        match self.receiver.poll_recv(context) {
            task::Poll::Ready(Some(event)) => return task::Poll::Ready(Some(event)),
            task::Poll::Pending => return task::Poll::Pending,
            task::Poll::Ready(None) => {}
        }
        let Some(disconnected) = &mut self.disconnected else {
            return task::Poll::Ready(None);
        };
        match Pin::new(disconnected).poll(context) {
            task::Poll::Ready(Ok(events)) => {
                self.disconnected = None;
                self.pending = events;
                task::Poll::Ready(self.pending.pop_front())
            }
            task::Poll::Ready(Err(_)) => {
                self.disconnected = None;
                task::Poll::Ready(None)
            }
            task::Poll::Pending => task::Poll::Pending,
        }
    }
}

/// A future that owns and drives an IRC session and its transport.
#[must_use = "a Driver does nothing unless awaited or spawned"]
pub struct Driver {
    #[cfg(not(browser))]
    inner: Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>,
    #[cfg(browser)]
    inner: Pin<Box<dyn Future<Output = Result<(), Error>>>>,
}

impl Driver {
    #[cfg(all(test, not(browser)))]
    fn with_stream<S>(stream: S, session: Session) -> (Client, Events, Self)
    where
        S: ConnectionIo + 'static,
    {
        Self::with_stream_and_capacity(stream, session, DEFAULT_CHANNEL_CAPACITY)
    }

    #[cfg(all(test, not(browser)))]
    fn with_stream_and_capacity<S>(
        stream: S,
        session: Session,
        capacity: usize,
    ) -> (Client, Events, Self)
    where
        S: ConnectionIo + 'static,
    {
        Self::with_stream_and_registration(
            stream,
            session,
            capacity,
            None,
            PostRegistration::default(),
        )
    }

    pub(crate) fn with_stream_and_registration<S>(
        stream: S,
        session: Session,
        capacity: usize,
        registration: Option<RegistrationSender>,
        defaults: PostRegistration,
    ) -> (Client, Events, Self)
    where
        S: ConnectionIo + 'static,
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

    pub(crate) fn failed(
        session: &Session,
        capacity: usize,
        registration: Option<RegistrationSender>,
        error: Error,
    ) -> (Client, Events, Self) {
        let channels = channels(capacity, session);
        let inner = Box::pin(async move {
            if let Some(registration) = registration {
                _ = registration.send(Err(error));
                Ok(())
            } else {
                Err(error)
            }
        });
        (channels.client, channels.events, Self { inner })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_plan(
        plan: Result<ConnectionPlan, InvalidInput>,
        session: Session,
        capacity: usize,
        registration: Option<RegistrationSender>,
        defaults: PostRegistration,
        reconnect: Reconnect,
        sts_store: Box<dyn StsStore + Send>,
    ) -> (Client, Events, Self) {
        Self::with_connector(
            plan,
            session,
            capacity,
            registration,
            defaults,
            reconnect,
            sts_store,
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
        plan: Result<ConnectionPlan, InvalidInput>,
        session: Session,
        capacity: usize,
        registration: Option<RegistrationSender>,
        defaults: PostRegistration,
        reconnect: Reconnect,
        sts_store: Box<dyn StsStore + Send>,
        connector: Connector,
    ) -> (Client, Events, Self) {
        Self::with_connector_and_entropy(
            plan,
            session,
            capacity,
            registration,
            defaults,
            reconnect,
            sts_store,
            connector,
            Box::new(crate::entropy::OsEntropy::next_u64),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_connector_and_entropy(
        plan: Result<ConnectionPlan, InvalidInput>,
        session: Session,
        capacity: usize,
        registration: Option<RegistrationSender>,
        defaults: PostRegistration,
        reconnect: Reconnect,
        sts_store: Box<dyn StsStore + Send>,
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
            sts_store,
            connector,
            entropy,
        ));
        (channels.client, channels.events, Self { inner })
    }
}

pub(crate) fn channels(capacity: usize, session: &Session) -> Channels {
    let (command_sender, command_receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
    let (event_sender, event_receiver) = mpsc::channel(capacity);
    let (disconnect_sender, disconnect_receiver) = oneshot::channel();
    let connected = Arc::new(AtomicBool::new(false));
    let state = Arc::new(RwLock::new(Arc::new(State::from_session(session, false))));
    let client = Client {
        commands: command_sender,
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
            pending: VecDeque::new(),
        },
        commands: command_receiver,
        event_sink: events,
    }
}

impl Future for Driver {
    type Output = Result<(), Error>;

    fn poll(mut self: Pin<&mut Self>, context: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        self.inner.as_mut().poll(context)
    }
}

#[allow(clippy::too_many_arguments)]
async fn run<S>(
    stream: S,
    mut session: Session,
    mut commands: mpsc::Receiver<Command>,
    events: EventSink,
    mut registration: RegistrationWait,
    mut defaults: PostRegistration,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut registered = false;
    let end = run_connection(
        stream,
        &mut session,
        &mut commands,
        &events,
        &mut registration,
        &mut defaults,
        &mut registered,
        None,
        None,
        0,
        VecDeque::new(),
        &events.connected,
        false,
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
    pending_events: VecDeque<Event>,
    reconnect_now: bool,
}

#[derive(Default)]
struct StsActions {
    upgrade: Option<u16>,
}

struct PendingWrite {
    buffer: Vec<u8>,
    offset: usize,
    token: Option<TransmitToken>,
}

impl ConnectionEnd {
    fn registration_error(&self) -> Error {
        match &self.error {
            Error::Disconnected(crate::Disconnected {
                cause: crate::DisconnectCause::Server { .. },
                ..
            }) => Error::Registration(crate::RegistrationError::Rejected {
                message: self.reason.clone(),
            }),
            error => error.clone(),
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn run_connection<S>(
    mut stream: S,
    session: &mut Session,
    commands: &mut mpsc::Receiver<Command>,
    events: &EventSink,
    registration: &mut RegistrationWait,
    defaults: &mut PostRegistration,
    registered: &mut bool,
    connection: Option<Connection>,
    mut sts_store: Option<Box<dyn StsStore + Send>>,
    sts_port: u16,
    mut pending_events: VecDeque<Event>,
    connected: &AtomicBool,
    reconnectable: bool,
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
    let mut commands_open = true;
    let mut quit = None;
    let mut requests = BTreeMap::new();
    let mut transmits = BTreeMap::new();
    let mut join_keys = BTreeMap::new();
    let mut active_typing = BTreeMap::new();
    let mut sync_waiters = Vec::new();
    let mut catchups = BTreeMap::new();
    let mut registered_at = None;
    let mut sts = StsActions::default();
    let mut reconnect_now = false;
    let mut buffer = [0; 4096];
    let mut pending_write = None;
    let mut drain_after_timeout = false;
    let (reason, error, close_kind, quit_timed_out) = loop {
        drop_closed_waiters(&mut requests, &mut sync_waiters, registration);
        let write_deadline = [
            session.poll_timeout(),
            quit.as_ref().map(|quit: &Quit| quit.deadline),
        ]
        .into_iter()
        .flatten()
        .min();
        match drain(
            &mut stream,
            session,
            &mut pending_write,
            events,
            &mut pending_events,
            &mut requests,
            &mut transmits,
            &mut join_keys,
            &mut sync_waiters,
            &mut catchups,
            registration,
            defaults,
            registered,
            &mut registered_at,
            &mut sts,
            &mut sts_store,
            sts_port,
            quit.is_some(),
            drain_after_timeout,
            write_deadline,
        )
        .await
        {
            Ok(Some((kind, reason))) => {
                let error = if reason == SLOW_CONSUMER_REASON {
                    Error::disconnected_slow_consumer()
                } else {
                    Error::disconnected_close(kind, reason.clone())
                };
                break (reason, error, Some(kind), false);
            }
            Ok(None) => {}
            Err(error) => {
                let reason = error.to_string();
                break (reason, Error::connection_lost(), None, false);
            }
        }
        drain_after_timeout = false;
        let read_ahead = should_read_ahead(
            &requests,
            &sync_waiters,
            &catchups,
            registration,
            quit.is_some(),
        );
        let deadline = [
            session.poll_timeout(),
            quit.as_ref().map(|quit: &Quit| quit.deadline),
        ]
        .into_iter()
        .flatten()
        .min();
        tokio::select! {
            read = stream.read(&mut buffer), if pending_events.is_empty() || read_ahead => match read {
                Ok(0) => {
                    break (
                        "server closed the connection".to_owned(),
                        Error::connection_lost(),
                        None,
                        false,
                    );
                }
                Ok(count) => session.handle_bytes(now(), &buffer[..count]),
                Err(error) => {
                    let reason = error.to_string();
                    break (reason, Error::connection_lost(), None, false);
                }
            },
            permit = events.regular.clone().reserve_owned(), if !pending_events.is_empty() => match permit {
                Ok(permit) => {
                    _ = permit.send(pending_events.pop_front().expect("pending event is present"));
                }
                Err(_) => pending_events.clear(),
            },
            command = commands.recv(), if commands_open => match command {
                Some(Command::Send { message, result }) => {
                    match session.send_tracked(now(), message) {
                        Ok(token) => { transmits.insert(token, PendingTransmit::Empty(result)); }
                        Err(error) => { _ = result.send(Err(Error::build(error))); }
                    }
                }
                Some(Command::Request { message, matcher, result }) => {
                    let requested = match matcher {
                        Some(matcher) => session.request_with_matcher(now(), message, matcher),
                        None if session.caps().contains_key(crate::cap::LABELED_RESPONSE) => {
                            session.request_silent(now(), message)
                        }
                        None => {
                            let reason = session
                                .supports(crate::Feature::LabeledResponse)
                                .expect_err("labeled-response is unavailable");
                            _ = result.send(Err(Error::Unsupported(crate::Unsupported::new(
                                crate::Feature::LabeledResponse,
                                reason,
                            ))));
                            continue;
                        }
                    };
                    match requested {
                        Ok(id) => {
                            requests.insert(id, PendingRequest::Response(result));
                        }
                        Err(error) => {
                            _ = result.send(Err(Error::request(error)));
                        }
                    }
                }
                Some(Command::Join { channel, key, result }) => {
                    let params = match &key {
                        Some(key) => {
                            let Some(key) = key.expose() else {
                                _ = result.send(Err(InvalidInput::BinarySecret.into()));
                                continue;
                            };
                            vec![channel.to_string(), key.to_owned()]
                        }
                        None => vec![channel.to_string()],
                    };
                    match session.request_with_matcher(
                        now(),
                        Line::new("JOIN", params),
                        ResponseMatcher::Join(channel.to_string()),
                    ) {
                        Ok(id) => {
                            requests.insert(id, PendingRequest::Response(result));
                            join_keys.insert(id, (channel, key));
                        }
                        Err(error) => { _ = result.send(Err(Error::request(error))); }
                    }
                }
                Some(Command::CoreRequest { request, result }) => {
                    match request.start(session) {
                        Ok(id) => { requests.insert(id, PendingRequest::Response(result)); }
                        Err(error) => { _ = result.send(Err(Error::request(error))); }
                    }
                }
                Some(Command::History { query, result }) => {
                    match session.chathistory(now(), query) {
                        Ok(id) => { requests.insert(id, PendingRequest::Response(result)); }
                        Err(error) => { _ = result.send(Err(Error::request(error))); }
                    }
                }
                Some(Command::MarkRead { target, time, result }) => {
                    match session.mark_read(now(), &target, time) {
                        Ok(id) => { requests.insert(id, PendingRequest::Response(result)); }
                        Err(error) => { _ = result.send(Err(Error::request(error))); }
                    }
                }
                Some(Command::ReadMarker { target, result }) => {
                    match session.read_marker(now(), &target) {
                        Ok(id) => { requests.insert(id, PendingRequest::Response(result)); }
                        Err(error) => { _ = result.send(Err(Error::request(error))); }
                    }
                }
                Some(Command::MonitorAdd { nicks, result }) => {
                    session.monitor_add(nicks);
                    _ = result.send(Ok(()));
                }
                Some(Command::MonitorRemove { nicks, result }) => {
                    session.monitor_remove(nicks);
                    _ = result.send(Ok(()));
                }
                Some(Command::MonitorClear { result }) => {
                    session.monitor_clear();
                    _ = result.send(Ok(()));
                }
                Some(Command::SendText { kind, target, text, tags, mut fallbacks, result }) => {
                    let tags: Vec<_> = tags.iter().map(|(key, value)| (key.as_str(), value.as_str())).collect();
                    match session.send_text_detailed(now(), kind, &target, &text, &tags) {
                        Ok(sent) => {
                            if sent.split {
                                fallbacks.push(crate::Fallback::SplitIntoLines);
                            }
                            if let Some(id) = sent.request {
                                requests.insert(id, PendingRequest::Text { target, fallbacks, result });
                            } else if let Some(token) = sent.transmit {
                                transmits.insert(token, PendingTransmit::Text { target, fallbacks, result });
                            }
                        }
                        Err(error) => {
                            _ = result.send(Err(Error::request(error)));
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
                    if send {
                        let value = match state {
                            Typing::Active => "active",
                            Typing::Paused => "paused",
                            Typing::Done => "done",
                        };
                        match session.send_tracked(
                            current,
                            Line::tagmsg(target).with_tag("+typing", value),
                        ) {
                            Ok(token) => { transmits.insert(token, PendingTransmit::Empty(result)); }
                            Err(error) => { _ = result.send(Err(Error::build(error))); }
                        }
                    } else {
                        _ = result.send(Ok(()));
                    }
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
                Some(Command::Reconnect { result }) => {
                    if reconnectable {
                        _ = result.send(Ok(()));
                        reconnect_now = true;
                        break (
                            "reconnect requested".to_owned(),
                            Error::disconnected_requested(),
                            None,
                            false,
                        );
                    }
                    _ = result.send(Err(Error::connection_lost()));
                }
                Some(Command::Quit { message, result }) => {
                    request_quit(session, &mut quit, message, Some(result));
                }
                None => {
                    commands_open = false;
                    request_quit(
                        session,
                        &mut quit,
                        Line::new("QUIT", Vec::<String>::new()),
                        None,
                    );
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
                drain_after_timeout = !pending_events.is_empty();
            },
        }
    };
    let disconnected = disconnected_event(&error, None);
    session.handle_disconnected(now());
    while let Some(core_event) = session.poll_event() {
        let core_event = event::from_core(core_event, session.isupport().casemapping());
        if let event::CoreEvent::RequestAborted { id, transmitted } = core_event {
            join_keys.remove(&id);
            if let Some(result) = requests.remove(&id) {
                let error = if transmitted {
                    Error::Interrupted(disconnected.clone())
                } else {
                    Error::disconnected(disconnected.clone())
                };
                result.fail(error);
            }
        }
    }
    for (_, result) in transmits {
        result.fail(Error::disconnected(disconnected.clone()));
    }
    for (_, result) in requests {
        result.fail(Error::disconnected(disconnected.clone()));
    }
    for (_, result) in sync_waiters {
        _ = result.send(Err(Error::disconnected(disconnected.clone())));
    }
    connected.store(false, Ordering::Relaxed);
    publish_state(events, session, false);
    close_stream_before(&mut stream, quit.as_ref().map(|quit: &Quit| quit.deadline)).await;
    ConnectionEnd {
        reason,
        error,
        close_kind,
        quit,
        quit_timed_out,
        registered_at,
        sts,
        sts_store,
        pending_events,
        reconnect_now,
    }
}

fn request_quit(
    session: &mut Session,
    quit: &mut Option<Quit>,
    message: Line,
    result: Option<oneshot::Sender<Result<(), Error>>>,
) {
    if let Some(quit) = quit {
        if let Some(result) = result {
            quit.results.push(result);
        }
        return;
    }
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
                _ = result.send(Err(Error::build(error)));
            }
        }
    }
}

async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => crate::rt::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

async fn close_stream_before<S>(stream: &mut S, deadline: Option<Instant>)
where
    S: AsyncWrite + Unpin,
{
    tokio::select! {
        () = async {
            _ = stream.flush().await;
            _ = stream.shutdown().await;
        } => {}
        () = wait_until(deadline) => {}
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
#[cfg_attr(browser, allow(unused_variables))]
async fn drain<S>(
    stream: &mut S,
    session: &mut Session,
    pending_write: &mut Option<PendingWrite>,
    events: &EventSink,
    pending_events: &mut VecDeque<Event>,
    requests: &mut BTreeMap<RequestId, PendingRequest>,
    transmits: &mut BTreeMap<TransmitToken, PendingTransmit>,
    join_keys: &mut BTreeMap<RequestId, (crate::ChannelName, Option<crate::Secret>)>,
    sync_waiters: &mut Vec<(String, oneshot::Sender<Result<(), Error>>)>,
    catchups: &mut BTreeMap<RequestId, Catchup>,
    registration: &mut RegistrationWait,
    defaults: &mut PostRegistration,
    registered: &mut bool,
    registered_at: &mut Option<Instant>,
    sts: &mut StsActions,
    sts_store: &mut Option<Box<dyn StsStore + Send>>,
    sts_port: u16,
    quitting: bool,
    after_timeout: bool,
    deadline: Option<Instant>,
) -> io::Result<Option<(papo_core::CloseKind, String)>>
where
    S: AsyncWrite + Unpin,
{
    publish_state(events, session, true);
    if !drain_transmit(stream, session, pending_write, transmits, deadline).await? {
        return Ok(None);
    }
    drop_closed_waiters(requests, sync_waiters, registration);
    let read_ahead = should_read_ahead(requests, sync_waiters, catchups, registration, quitting);
    while (pending_events.is_empty() || read_ahead || after_timeout)
        && let Some(core_event) = session.poll_event()
    {
        match event::from_core(core_event, session.isupport().casemapping()) {
            event::CoreEvent::Forward(mut event) => {
                event.mark_self(session.nick(), session.isupport().casemapping());
                if let Event::ChannelSynced(synced) = &event {
                    resolve_sync_waiters(session, sync_waiters, &synced.channel);
                }
                update_rejoin_channels(session, defaults, &event);
                if matches!(
                    event,
                    Event::AccountRegistration(
                        crate::AccountRegistration::VerificationRequired { .. }
                    )
                ) {
                    registration.succeeded();
                }
                if !enqueue_event(events, pending_events, event) {
                    return Ok(Some((
                        papo_core::CloseKind::Other,
                        SLOW_CONSUMER_REASON.to_owned(),
                    )));
                }
            }
            event::CoreEvent::Registered(welcome) => {
                after_registration(session, &welcome, defaults)?;
                registration.succeeded();
                *registered_at = Some(now());
                let reconnected = *registered;
                if reconnected {
                    begin_catchup(session, defaults, catchups);
                } else {
                    *registered = true;
                }
                let event = Event::Registered(event::Registered {
                    nick: welcome.nick,
                    text: welcome.text,
                    caps: welcome.caps,
                    reconnected,
                });
                if !enqueue_event(events, pending_events, event) {
                    return Ok(Some((
                        papo_core::CloseKind::Other,
                        SLOW_CONSUMER_REASON.to_owned(),
                    )));
                }
            }
            event::CoreEvent::Line(message) => {
                drop_bad_rejoin_key(session, defaults, &message.as_ref());
                let event =
                    crate::Message::from_core(message, session.isupport(), crate::rt::now_wall())
                        .map_or_else(
                            |message| {
                                Event::Raw(crate::RawLine::from_proto(
                                    &message,
                                    session.isupport().casemapping(),
                                ))
                            },
                            |message| Event::Message(Box::new(message)),
                        );
                if !enqueue_event(events, pending_events, event) {
                    return Ok(Some((
                        papo_core::CloseKind::Other,
                        SLOW_CONSUMER_REASON.to_owned(),
                    )));
                }
            }
            event::CoreEvent::History { target, items, end } => {
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
                    state: Arc::clone(&events.state),
                };
                let items = items
                    .into_iter()
                    .map(|item| {
                        crate::ext::history::history_item(
                            item,
                            &client,
                            crate::rt::now_wall(),
                            crate::Origin::Replay,
                        )
                    })
                    .collect();
                let event = Event::History(Box::new(crate::HistoryPage {
                    target: target.into(),
                    items,
                    end,
                }));
                if !enqueue_event(events, pending_events, event) {
                    return Ok(Some((
                        papo_core::CloseKind::Other,
                        SLOW_CONSUMER_REASON.to_owned(),
                    )));
                }
            }
            event::CoreEvent::Raw(message) => {
                drop_bad_rejoin_key(session, defaults, &message.as_ref());
                let event = crate::Message::new(
                    message,
                    session.isupport(),
                    session.nick(),
                    session.caps(),
                    crate::rt::now_wall(),
                )
                .map_or_else(
                    |message| {
                        Event::Raw(crate::RawLine::from_proto(
                            &message,
                            session.isupport().casemapping(),
                        ))
                    },
                    |message| Event::Message(Box::new(message)),
                );
                if !enqueue_event(events, pending_events, event) {
                    return Ok(Some((
                        papo_core::CloseKind::Other,
                        SLOW_CONSUMER_REASON.to_owned(),
                    )));
                }
            }
            event::CoreEvent::Response(response) => {
                for line in &response.lines {
                    drop_bad_rejoin_key(session, defaults, &line.as_ref());
                }
                if response.error().is_none()
                    && let Some((channel, key)) = join_keys.remove(&response.id)
                {
                    remember_rejoin(session, defaults, Autojoin::new(channel.into_string(), key));
                } else {
                    join_keys.remove(&response.id);
                }
                if let Some(catchup) = catchups.remove(&response.id) {
                    if let Some(event) =
                        advance_catchup(session, events, catchups, defaults, catchup, response)
                        && !enqueue_event(events, pending_events, event)
                    {
                        return Ok(Some((
                            papo_core::CloseKind::Other,
                            SLOW_CONSUMER_REASON.to_owned(),
                        )));
                    }
                } else if let Some(result) = requests.remove(&response.id) {
                    result.respond(response);
                }
            }
            event::CoreEvent::RequestUntracked(id) => {
                join_keys.remove(&id);
                requests.remove(&id);
            }
            event::CoreEvent::RequestTimedOut(id) => {
                join_keys.remove(&id);
                if let Some(result) = requests.remove(&id) {
                    result.fail(Error::Timeout);
                }
            }
            event::CoreEvent::RequestAborted { id, transmitted } => {
                join_keys.remove(&id);
                if let Some(result) = requests.remove(&id) {
                    let disconnected =
                        crate::Disconnected::new(crate::DisconnectCause::ConnectionLost, None);
                    result.fail(if transmitted {
                        Error::Interrupted(disconnected)
                    } else {
                        Error::disconnected(disconnected)
                    });
                }
            }
            event::CoreEvent::TransmitAborted(token) => {
                if let Some(result) = transmits.remove(&token) {
                    result.fail(Error::connection_lost());
                }
            }
            event::CoreEvent::StsUpgrade(port) => sts.upgrade = Some(port),
            event::CoreEvent::StsPolicy(action) => {
                #[cfg(not(browser))]
                if let Some(store) = sts_store {
                    apply_sts_action(store.as_mut(), sts_port, &action);
                }
                #[cfg(browser)]
                {
                    _ = action;
                }
            }
            event::CoreEvent::Close {
                kind: papo_core::CloseKind::PingTimeout,
                ..
            } if after_timeout => {
                return Ok(Some((
                    papo_core::CloseKind::Other,
                    SLOW_CONSUMER_REASON.to_owned(),
                )));
            }
            event::CoreEvent::Close { kind, reason } => return Ok(Some((kind, reason))),
            event::CoreEvent::RequestUnsupported { id, unsupported } => {
                join_keys.remove(&id);
                catchups.remove(&id);
                if let Some(result) = requests.remove(&id) {
                    result.fail(Error::Unsupported(unsupported));
                }
            }
            event::CoreEvent::Unknown => {}
        }
    }
    drain_transmit(stream, session, pending_write, transmits, deadline).await?;
    Ok(None)
}

fn should_read_ahead(
    requests: &BTreeMap<RequestId, PendingRequest>,
    sync_waiters: &[(String, oneshot::Sender<Result<(), Error>>)],
    catchups: &BTreeMap<RequestId, Catchup>,
    registration: &RegistrationWait,
    quitting: bool,
) -> bool {
    !requests.is_empty()
        || !sync_waiters.is_empty()
        || !catchups.is_empty()
        || registration.0.is_some()
        || quitting
}

async fn drain_transmit<S>(
    stream: &mut S,
    session: &mut Session,
    pending: &mut Option<PendingWrite>,
    transmits: &mut BTreeMap<TransmitToken, PendingTransmit>,
    deadline: Option<Instant>,
) -> io::Result<bool>
where
    S: AsyncWrite + Unpin,
{
    loop {
        if pending.is_none() {
            *pending = session.poll_transmit_tracked().map(|transmit| {
                let (buffer, token) = transmit.into_parts();
                PendingWrite {
                    buffer,
                    offset: 0,
                    token,
                }
            });
        }
        let Some(write) = pending.as_mut() else {
            return Ok(true);
        };
        if !write_before(stream, write, deadline).await? {
            return Ok(false);
        }
        let written = pending.take().expect("completed write remains pending");
        if let Some(token) = written.token
            && let Some(result) = transmits.remove(&token)
        {
            result.complete();
        }
    }
}

async fn write_before<S>(
    stream: &mut S,
    pending: &mut PendingWrite,
    deadline: Option<Instant>,
) -> io::Result<bool>
where
    S: AsyncWrite + Unpin,
{
    while pending.offset < pending.buffer.len() {
        let written = tokio::select! {
            result = stream.write(&pending.buffer[pending.offset..]) => result?,
            () = wait_until(deadline) => return Ok(false),
        };
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        pending.offset += written;
    }
    Ok(true)
}

fn update_rejoin_channels(session: &Session, defaults: &mut PostRegistration, event: &Event) {
    let casemapping = session.isupport().casemapping();
    match event {
        Event::Joined(joined) if joined.is_self => {
            if !defaults
                .channels
                .iter()
                .any(|entry| casemapping.eq(&entry.channel, &joined.channel))
            {
                defaults
                    .channels
                    .push(Autojoin::new(joined.channel.as_ref(), None));
            }
        }
        Event::Parted(parted) if parted.is_self => {
            defaults
                .channels
                .retain(|entry| !casemapping.eq(&entry.channel, &parted.channel));
        }
        Event::Kicked(kicked) if kicked.is_self => {
            defaults
                .channels
                .retain(|entry| !casemapping.eq(&entry.channel, &kicked.channel));
        }
        _ => {}
    }
}

fn remember_rejoin(session: &Session, defaults: &mut PostRegistration, entry: Autojoin) {
    let casemapping = session.isupport().casemapping();
    defaults
        .channels
        .retain(|configured| !casemapping.eq(&configured.channel, &entry.channel));
    defaults.channels.push(entry);
}

fn drop_bad_rejoin_key(
    session: &Session,
    defaults: &mut PostRegistration,
    message: &papo_proto::LineRef<'_>,
) {
    if message.command() != ProtoCommand::Numeric(Numeric::ERR_BADCHANNELKEY) {
        return;
    }
    let Some(channel) = message.param(1) else {
        return;
    };
    let casemapping = session.isupport().casemapping();
    defaults
        .channels
        .retain(|entry| !casemapping.eq(&entry.channel, channel));
}

fn dedupe_autojoin(session: &Session, defaults: &mut PostRegistration) {
    let casemapping = session.isupport().casemapping();
    let mut deduplicated = Vec::with_capacity(defaults.channels.len());
    for entry in defaults.channels.drain(..) {
        deduplicated
            .retain(|configured: &Autojoin| !casemapping.eq(&configured.channel, &entry.channel));
        deduplicated.push(entry);
    }
    defaults.channels = deduplicated;
}

fn enqueue_event(events: &EventSink, pending: &mut VecDeque<Event>, event: Event) -> bool {
    let event = if pending.is_empty() {
        match events.regular.try_send(event) {
            Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => return true,
            Err(mpsc::error::TrySendError::Full(event)) => event,
        }
    } else {
        event
    };
    if pending.len() >= events.regular.max_capacity() {
        return false;
    }
    pending.push_back(event);
    true
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

fn drop_closed_waiters(
    requests: &mut BTreeMap<RequestId, PendingRequest>,
    sync_waiters: &mut Vec<(String, oneshot::Sender<Result<(), Error>>)>,
    registration: &mut RegistrationWait,
) {
    requests.retain(|_, request| !request.is_closed());
    sync_waiters.retain(|(_, result)| !result.is_closed());
    if registration
        .0
        .as_ref()
        .is_some_and(oneshot::Sender::is_closed)
    {
        registration.0 = None;
    }
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
    defaults: &mut PostRegistration,
) -> Result<(), io::Error> {
    dedupe_autojoin(session, defaults);
    if defaults.bot
        && let Some(mode) = welcome.isupport.bot_mode()
    {
        session
            .send(
                now(),
                Line::new(
                    "MODE",
                    [welcome.nick.as_ref().to_owned(), format!("+{mode}")],
                ),
            )
            .map_err(io::Error::other)?;
    }
    if should_rejoin(session) {
        for channel in &defaults.channels {
            let params = match &channel.key {
                Some(key) => {
                    let key = key.expose().ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "channel key is not text")
                    })?;
                    vec![channel.channel.clone(), key.to_owned()]
                }
                None => vec![channel.channel.clone()],
            };
            session
                .send(now(), Line::new("JOIN", params))
                .map_err(io::Error::other)?;
        }
    }
    Ok(())
}

fn should_rejoin(session: &Session) -> bool {
    #[cfg(not(feature = "unstable"))]
    let _ = session;
    #[cfg(feature = "unstable")]
    return !session.server_rejoins_channels();
    #[cfg(not(feature = "unstable"))]
    true
}

fn finish(events: EventSink, end: ConnectionEnd, _will_retry: bool) -> Result<(), Error> {
    let mut pending = end.pending_events;
    let disconnected = if end.quit.is_some() {
        crate::Disconnected::new(
            crate::DisconnectCause::Requested {
                quit_unacknowledged: end.quit_timed_out,
            },
            None,
        )
    } else {
        disconnected_event(&end.error, None)
    };
    pending.push_back(Event::Disconnected(disconnected));
    events.finish(pending);
    if let Some(quit) = end.quit {
        for response in quit.results {
            _ = response.send(Ok(()));
        }
        Ok(())
    } else {
        Err(end.error)
    }
}

fn disconnected_event(error: &Error, retry_in: Option<Duration>) -> crate::Disconnected {
    let cause = match error {
        Error::Disconnected(disconnected) => disconnected.cause.clone(),
        Error::Connect(error) => crate::DisconnectCause::Connect(error.clone()),
        Error::Registration(error) => crate::DisconnectCause::Registration(error.clone()),
        _ => crate::DisconnectCause::ConnectionLost,
    };
    crate::Disconnected::new(cause, retry_in)
}

fn now() -> Instant {
    crate::rt::now_mono()
}

#[cfg(all(test, not(browser)))]
pub(crate) mod test_support;

#[cfg(all(test, not(browser)))]
mod proptests;

#[cfg(all(test, not(browser)))]
mod tests;
