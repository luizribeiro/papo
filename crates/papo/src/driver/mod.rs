#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)
)]

mod catchup;
#[allow(
    clippy::expect_used,
    reason = "connection phases, STS stores, and queued events are validated immediately before use"
)]
mod connect;
#[cfg(feature = "unstable")]
pub(crate) mod unstable;

use catchup::{Catchup, CatchupCursor, advance_catchup, begin_catchup, server_restores_channels};

use connect::apply_sts_action;
use connect::{open_connection, run_plan};

use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
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
use papo_proto::{Command as ProtoCommand, LimitValue, Line, Numeric, Typing};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

use crate::{
    Error, Event, InvalidInput, Reconnect,
    address::ServerAddress,
    client::{Client, Sent},
    event_impl as event,
    rt::{BoxFuture, Io},
    state::State,
    transport::TlsConfig,
};

pub(crate) const DEFAULT_CHANNEL_CAPACITY: usize = 1024;
const QUIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const STABLE_CONNECTION: Duration = Duration::from_secs(60);
const MAX_REJOIN_CHANNELS: usize = 10_000;

type BoxConnection = Box<dyn Io>;
type ConnectFuture = BoxFuture<Result<BoxConnection, Error>>;
type CompositeFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
pub(crate) type Connector = Arc<
    dyn Fn(ConnectionPlan, mpsc::UnboundedSender<papo_core::ConnectPhase>) -> ConnectFuture
        + Send
        + Sync,
>;
type BackoffEntropy = Box<dyn FnMut() -> u64 + Send>;

pub(crate) enum CoreRequest {
    Register {
        account: Option<String>,
        email: Option<String>,
        password: papo_core::Secret,
    },
    Verify {
        account: String,
        code: papo_core::Secret,
    },
    Oper {
        name: String,
        password: papo_core::Secret,
    },
    #[cfg(feature = "unstable")]
    Unstable(unstable::UnstableRequest),
    Metadata(papo_core::MetadataCommand),
    Rename {
        old: String,
        new: String,
        reason: String,
    },
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
                session.create_account(now(), account.as_deref(), email.as_deref(), password)?;
            }
            Self::Verify { account, code } => session.verify(now(), &account, code)?,
            Self::Oper { name, password } => {
                session.oper(now(), &name, &password)?;
            }
            #[cfg(feature = "unstable")]
            Self::Unstable(request) => return request.start(session),
            Self::Metadata(command) => return session.metadata(now(), command),
            Self::Rename { old, new, reason } => return session.rename(now(), &old, &new, &reason),
        }
        Ok(before)
    }
}

pub(crate) enum Command {
    Composite {
        task: CompositeFuture,
    },
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
    Part {
        channel: crate::ChannelName,
        reason: String,
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
    DirectAcquire {
        nick: String,
        account: Option<crate::Account>,
        result: oneshot::Sender<Result<papo_core::RecipientGuard, Error>>,
    },
    DirectRefresh {
        nick: String,
        account: crate::Account,
        result: oneshot::Sender<Result<papo_core::RecipientGuard, Error>>,
    },
    DirectRelease {
        recipient: papo_core::RecipientGuard,
    },
    SendText {
        kind: TextKind,
        target: String,
        conversation: crate::Conversation,
        status: Option<crate::Prefix>,
        receipt: crate::message_ref::Receipt,
        text: String,
        tags: Vec<(String, String)>,
        fallbacks: Vec<crate::Fallback>,
        action: bool,
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
    AutoJoin {
        channel: crate::ChannelName,
    },
    Text {
        conversation: crate::Conversation,
        status: Option<crate::Prefix>,
        receipt: crate::message_ref::Receipt,
        fallbacks: Vec<crate::Fallback>,
        remaining: usize,
        confirmations: Vec<papo_core::SentInfo>,
        result: oneshot::Sender<Result<Sent, Error>>,
    },
}

impl PendingRequest {
    fn is_closed(&self) -> bool {
        match self {
            Self::Response(result) => result.is_closed(),
            Self::AutoJoin { .. } => false,
            Self::Text { result, .. } => result.is_closed(),
        }
    }

    fn is_text(&self) -> bool {
        matches!(self, Self::Text { .. })
    }

    fn respond(self, response: Response, resolved_sent: &mut VecDeque<String>) -> Option<Self> {
        match self {
            Self::Response(result) => {
                _ = result.send(Ok(response));
                None
            }
            Self::AutoJoin { .. } => None,
            Self::Text {
                conversation,
                status,
                receipt,
                fallbacks,
                remaining,
                mut confirmations,
                result,
            } => {
                if let Some(error) = response.error() {
                    _ = result.send(Err(Error::server(error)));
                    return None;
                }
                confirmations.extend(response.sent());
                if remaining > 1 {
                    return Some(Self::Text {
                        conversation,
                        status,
                        receipt,
                        fallbacks,
                        remaining: remaining - 1,
                        confirmations,
                        result,
                    });
                }
                remember_resolved_sent(resolved_sent, &confirmations);
                _ = result.send(Ok(Sent::from_confirmations(
                    &confirmations,
                    conversation,
                    status,
                    receipt,
                    fallbacks,
                )));
                None
            }
        }
    }

    fn fail(self, error: Error) {
        match self {
            Self::Response(result) => _ = result.send(Err(error)),
            Self::AutoJoin { .. } => {}
            Self::Text { result, .. } => _ = result.send(Err(error)),
        }
    }
}

enum PendingTransmit {
    Empty(oneshot::Sender<Result<(), Error>>),
    Text {
        conversation: crate::Conversation,
        status: Option<crate::Prefix>,
        receipt: crate::message_ref::Receipt,
        fallbacks: Vec<crate::Fallback>,
        result: oneshot::Sender<Result<Sent, Error>>,
    },
}

impl PendingTransmit {
    fn complete(self) {
        match self {
            Self::Empty(result) => _ = result.send(Ok(())),
            Self::Text {
                conversation,
                status,
                receipt,
                fallbacks,
                result,
            } => {
                _ = result.send(Ok(Sent::without_echo(
                    conversation,
                    status,
                    receipt,
                    fallbacks,
                )));
            }
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
    deadline: Option<Instant>,
    results: Vec<oneshot::Sender<Result<(), Error>>>,
}

const PRECONNECT_EVENT_CAPACITY: usize = 32;

struct EventReceiver {
    inner: Mutex<mpsc::Receiver<Event>>,
    capacity: usize,
    connecting: AtomicBool,
}

impl EventReceiver {
    fn poll_recv(&self, context: &mut task::Context<'_>) -> task::Poll<Option<Event>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .poll_recv(context)
    }

    #[allow(dead_code)]
    fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    #[allow(dead_code)]
    fn max_capacity(&self) -> usize {
        self.capacity
    }

    fn discard_oldest_if_full(&self) -> bool {
        let mut receiver = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let capacity = if self.connecting.load(Ordering::Relaxed) {
            receiver.max_capacity()
        } else {
            self.capacity
        };
        receiver.len() >= capacity && receiver.try_recv().is_ok()
    }

    fn finish_connecting(&self) {
        self.connecting.store(false, Ordering::Relaxed);
    }
}

struct EventSink {
    regular: mpsc::Sender<Event>,
    receiver: Arc<EventReceiver>,
    dropped: Arc<AtomicU64>,
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

pub(crate) type ConnectSender = oneshot::Sender<Result<(), Error>>;

#[derive(Default)]
struct ConnectWait(Option<ConnectSender>);

impl ConnectWait {
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
pub(crate) struct PostConnect {
    pub(crate) bot: bool,
    pub(crate) channels: Vec<Autojoin>,
    pub(crate) parted_while_disconnected: Vec<crate::ChannelName>,
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
/// Delivery is bounded without blocking socket reads. When the buffer is full, the oldest events
/// are discarded and one [`Event::Lagged`] reports how many were lost. Move slow work out of the
/// event loop or raise [`Builder::event_capacity`](crate::Builder::event_capacity) when every
/// event matters.
pub struct Events {
    _commands: mpsc::Sender<Command>,
    receiver: Arc<EventReceiver>,
    dropped: Arc<AtomicU64>,
    disconnected: Option<oneshot::Receiver<VecDeque<Event>>>,
    pending: VecDeque<Event>,
}

impl std::fmt::Debug for Events {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("Events", formatter)
    }
}

impl Events {
    /// Waits for the next event, or returns `None` after the driver stops.
    ///
    /// This method is cancel-safe: cancelling the future does not consume an event.
    pub async fn next(&mut self) -> Option<Event> {
        let next = std::future::poll_fn(|context| Pin::new(&mut *self).poll_next(context));
        #[cfg(all(test, not(browser)))]
        let next = test_support::within(next);
        next.await
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
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        if dropped != 0 {
            return task::Poll::Ready(Some(Event::Lagged(crate::event::Lagged { dropped })));
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
///
/// Dropping the driver ends the associated [`Events`] stream without emitting a final
/// [`Event::Disconnected`](crate::Event::Disconnected).
#[must_use = "a Driver does nothing unless awaited or spawned"]
pub struct Driver {
    inner: BoxFuture<Result<(), Error>>,
}

impl std::fmt::Debug for Driver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("Driver", formatter)
    }
}

impl Driver {
    pub(crate) fn with_stream_and_connect<S>(
        stream: S,
        session: Session,
        capacity: usize,
        registration: Option<ConnectSender>,
        defaults: PostConnect,
    ) -> (Client, Events, Self)
    where
        S: Io + 'static,
    {
        let channels = channels(capacity, &session);
        let inner = Box::pin(run(
            stream,
            session,
            channels.commands,
            channels.event_sink,
            ConnectWait(registration),
            defaults,
        ));
        (channels.client, channels.events, Self { inner })
    }

    pub(crate) fn failed(
        session: &Session,
        capacity: usize,
        registration: Option<ConnectSender>,
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
        registration: Option<ConnectSender>,
        defaults: PostConnect,
        reconnect: Reconnect,
        connect_timeout: Duration,
        sts_store: Arc<dyn StsStore>,
        connector: Option<Connector>,
        network_builder: Arc<RwLock<crate::builder::NetworkBuilderSettings>>,
    ) -> (Client, Events, Self) {
        let connector = connector.unwrap_or_else(|| {
            Arc::new(|plan, progress| {
                Box::pin(async move {
                    open_connection(plan, progress)
                        .await
                        .map(|stream| Box::new(stream) as BoxConnection)
                })
            })
        });
        Self::with_connector_and_timeout(
            plan,
            session,
            capacity,
            registration,
            defaults,
            reconnect,
            connect_timeout,
            sts_store,
            connector,
            Box::new(crate::entropy::OsEntropy::next_u64),
            Some(network_builder),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_connector_and_timeout(
        plan: Result<ConnectionPlan, InvalidInput>,
        session: Session,
        capacity: usize,
        registration: Option<ConnectSender>,
        defaults: PostConnect,
        reconnect: Reconnect,
        connect_timeout: Duration,
        sts_store: Arc<dyn StsStore>,
        connector: Connector,
        entropy: BackoffEntropy,
        network_builder: Option<Arc<RwLock<crate::builder::NetworkBuilderSettings>>>,
    ) -> (Client, Events, Self) {
        let channels = channels(capacity, &session);
        let inner = Box::pin(run_plan(
            plan,
            session,
            channels.commands,
            channels.event_sink,
            ConnectWait(registration),
            defaults,
            reconnect,
            connect_timeout,
            sts_store,
            connector,
            entropy,
            network_builder,
        ));
        (channels.client, channels.events, Self { inner })
    }
}

pub(crate) fn channels(capacity: usize, session: &Session) -> Channels {
    let (command_sender, command_receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
    let queue_capacity = capacity
        .saturating_add(PRECONNECT_EVENT_CAPACITY)
        .min(tokio::sync::Semaphore::MAX_PERMITS);
    let (event_sender, event_receiver) = mpsc::channel(queue_capacity);
    let event_receiver = Arc::new(EventReceiver {
        inner: Mutex::new(event_receiver),
        capacity,
        connecting: AtomicBool::new(true),
    });
    let dropped = Arc::new(AtomicU64::new(0));
    let (disconnect_sender, disconnect_receiver) = oneshot::channel();
    let connected = Arc::new(AtomicBool::new(false));
    let state = Arc::new(RwLock::new(Arc::new(State::from_session(session, false))));
    let client = Client {
        commands: command_sender,
        state: Arc::clone(&state),
        origin: session.state().nick().origin(),
        network_builder: None,
    };
    let events = EventSink {
        regular: event_sender,
        receiver: Arc::clone(&event_receiver),
        dropped: Arc::clone(&dropped),
        disconnected: disconnect_sender,
        connected: Arc::clone(&connected),
        state,
        commands: client.commands.downgrade(),
    };
    let event_commands = client.commands.clone();
    Channels {
        client,
        events: Events {
            _commands: event_commands,
            receiver: event_receiver,
            dropped,
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
    mut registration: ConnectWait,
    mut defaults: PostConnect,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut registered = false;
    let mut end = run_connection(
        stream,
        &mut session,
        &mut commands,
        &events,
        &mut registration,
        &mut defaults,
        &mut registered,
        1,
        None,
        None,
        0,
        false,
        VecDeque::new(),
        &events.connected,
        false,
    )
    .await;
    if registration.0.is_some() {
        let error = end.login_error(true);
        registration.failed(error.clone());
        end.error = error;
    }
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
    sts_store: Option<Arc<dyn StsStore>>,
    pending_events: VecDeque<Event>,
    reconnect_now: bool,
}

#[derive(Default)]
struct StsActions {
    upgrade: Option<u16>,
}

struct PendingWrite {
    buffer: papo_proto::WireLine,
    offset: usize,
    token: Option<TransmitToken>,
}

impl ConnectionEnd {
    fn login_error(&self, awaiting_registration: bool) -> Error {
        match &self.error {
            Error::Disconnected(crate::Disconnected {
                cause: crate::DisconnectCause::Server { .. },
                ..
            }) => Error::Connect(crate::ConnectError::login(
                crate::ConnectErrorKind::Server,
                self.reason.clone(),
            )),
            Error::Disconnected(crate::Disconnected {
                cause: crate::DisconnectCause::ConnectionLost { message },
                ..
            }) if message == "server closed the connection" => Error::Connect(
                crate::ConnectError::login(crate::ConnectErrorKind::Server, self.reason.clone()),
            ),
            Error::Disconnected(_) if awaiting_registration => Error::Connect(
                crate::ConnectError::login(crate::ConnectErrorKind::Other, self.reason.clone()),
            ),
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
    registration: &mut ConnectWait,
    defaults: &mut PostConnect,
    registered: &mut bool,
    attempt: usize,
    connection: Option<Connection>,
    mut sts_store: Option<Arc<dyn StsStore>>,
    sts_port: u16,
    sts_websocket: bool,
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
    let mut resolved_sent = VecDeque::new();
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
            quit.as_ref().and_then(|quit: &Quit| quit.deadline),
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
            &mut resolved_sent,
            registration,
            defaults,
            registered,
            attempt,
            &mut registered_at,
            &mut sts,
            &mut sts_store,
            sts_port,
            sts_websocket,
            quit.is_some(),
            drain_after_timeout,
            write_deadline,
        )
        .await
        {
            Ok(Some((kind, reason))) => {
                let error = Error::disconnected_close(kind, reason.clone());
                break (reason, error, Some(kind), false);
            }
            Ok(None) => {}
            Err(error) => {
                let reason = error.to_string();
                break (
                    reason.clone(),
                    Error::connection_lost_with(reason),
                    None,
                    false,
                );
            }
        }
        drain_after_timeout = false;
        let deadline = [
            session.poll_timeout(),
            quit.as_ref().and_then(|quit: &Quit| quit.deadline),
        ]
        .into_iter()
        .flatten()
        .min();
        tokio::select! {
            read = stream.read(&mut buffer) => match read {
                Ok(0) => {
                    break (
                        "server closed the connection".to_owned(),
                        Error::connection_lost_with("server closed the connection"),
                        None,
                        false,
                    );
                }
                Ok(count) => {
                    if let Some(bytes) = buffer.get(..count) {
                        session.handle_bytes(now(), bytes);
                    }
                }
                Err(error) => {
                    let reason = error.to_string();
                    break (
                        reason.clone(),
                        Error::connection_lost_with(reason),
                        None,
                        false,
                    );
                }
            },
            command = commands.recv(), if commands_open => match command {
                Some(Command::Composite { task }) => {
                    _ = crate::rt::spawn(task);
                }
                Some(Command::Send { message, result }) => {
                    let typing_target = text_target(&message)
                        .map(|target| session.isupport().casemapping().fold(target).into_owned());
                    match session.send_tracked(now(), message) {
                        Ok(token) => {
                            if let Some(target) = typing_target {
                                active_typing.remove(&target);
                            }
                            transmits.insert(token, PendingTransmit::Empty(result));
                        }
                        Err(error) => { _ = result.send(Err(Error::build(error))); }
                    }
                }
                Some(Command::Request { message, matcher, result }) => {
                    let typing_target = text_target(&message)
                        .map(|target| session.isupport().casemapping().fold(target).into_owned());
                    let requested = match matcher {
                        Some(matcher) => session.request_with_matcher(now(), message, matcher),
                        None if session.caps().contains_key(&crate::cap::LABELED_RESPONSE) => {
                            session.request_silent(now(), message)
                        }
                        None => {
                            let Err(reason) = session.supports(crate::Feature::LabeledResponse)
                            else {
                                continue;
                            };
                            _ = result.send(Err(Error::Unsupported(crate::Unsupported::new(
                                crate::Feature::LabeledResponse,
                                reason,
                            ))));
                            continue;
                        }
                    };
                    match requested {
                        Ok(id) => {
                            if let Some(target) = typing_target {
                                active_typing.remove(&target);
                            }
                            requests.insert(id, PendingRequest::Response(result));
                        }
                        Err(error) => {
                            _ = result.send(Err(Error::request(error)));
                        }
                    }
                }
                Some(Command::Join { channel, key, result }) => {
                    defaults
                        .parted_while_disconnected
                        .retain(|parted| parted != channel.as_ref());
                    match session.join(now(), channel.as_ref(), key.as_ref()) {
                        Ok(id) => {
                            requests.insert(id, PendingRequest::Response(result));
                            join_keys.insert(id, (channel, key));
                        }
                        Err(error) => { _ = result.send(Err(Error::request(error))); }
                    }
                }
                Some(Command::Part { channel, reason, result }) => {
                    let mut params = vec![channel.to_string()];
                    if !reason.is_empty() {
                        params.push(reason);
                    }
                    match session.request_with_matcher(
                        now(),
                        Line::new("PART").with_params(params),
                        ResponseMatcher::Part(channel.to_string()),
                    ) {
                        Ok(id) => {
                            requests.insert(id, PendingRequest::Response(result));
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
                Some(Command::DirectAcquire { nick, account, result }) => {
                    let already_monitored = session
                        .state()
                        .monitored()
                        .any(|known| known == nick.as_str());
                    if !already_monitored
                        && let Some(LimitValue::Limited(limit)) = session.isupport().monitor()
                        && session.state().monitored().len() >= limit
                    {
                        _ = result.send(Err(Error::MonitorFull { limit }));
                        continue;
                    }
                    let recipient = session.__direct_acquire(
                        &nick,
                        None,
                        None,
                        account.unwrap_or(crate::Account::Unknown),
                    );
                    publish_state(events, session, true);
                    _ = result.send(Ok(recipient));
                }
                Some(Command::DirectRefresh { nick, account, result }) => {
                    let recipient = session.__direct_refresh(&nick, account);
                    publish_state(events, session, true);
                    _ = result.send(Ok(recipient));
                }
                Some(Command::DirectRelease { recipient }) => {
                    session.__direct_release(&recipient);
                    publish_state(events, session, true);
                }
                Some(Command::SendText { kind, target, conversation, status, receipt, text, tags, mut fallbacks, action, result }) => {
                    let tags: Vec<_> = tags.iter().map(|(key, value)| (key.as_str(), value.as_str())).collect();
                    let sent = if action {
                        session.send_action_detailed(now(), &target, &text, &tags)
                    } else {
                        session.send_text_detailed(now(), kind, &target, &text, &tags)
                    };
                    match sent {
                        Ok(sent) => {
                            active_typing.remove(
                                session.isupport().casemapping().fold(&target).as_ref(),
                            );
                            if sent.split {
                                fallbacks.push(crate::Fallback::SplitIntoLines);
                            }
                            if let Some(id) = sent.request {
                                requests.insert(id, PendingRequest::Text {
                                    conversation,
                                    status,
                                    receipt,
                                    fallbacks,
                                    remaining: sent.confirmations,
                                    confirmations: Vec::new(),
                                    result,
                                });
                            } else if let Some(token) = sent.transmit {
                                transmits.insert(token, PendingTransmit::Text {
                                    conversation,
                                    status,
                                    receipt,
                                    fallbacks,
                                    result,
                                });
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
                            papo_core::deadline(*last, Duration::from_secs(3))
                                .is_none_or(|deadline| current >= deadline)
                        });
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
                            Ok(token) => {
                                if state == Typing::Active {
                                    active_typing.insert(key, current);
                                } else {
                                    active_typing.remove(&key);
                                }
                                transmits.insert(token, PendingTransmit::Empty(result));
                            }
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
                        Line::new("QUIT"),
                        None,
                    );
                }
            },
            () = wait_until(deadline) => {
                let current = now();
                if quit
                    .as_ref()
                    .and_then(|quit| quit.deadline)
                    .is_some_and(|deadline| current >= deadline)
                {
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
        let core_event = event::from_core_with_receipt(
            core_event,
            session.isupport().casemapping(),
            session.state().nick().origin(),
            session.caps().contains_key(&crate::cap::ACCOUNT_TAG),
            can_prove_non_bot(session),
            crate::rt::now_wall(),
            Some(session.state()),
        );
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
    close_stream_before(
        &mut stream,
        quit.as_ref().and_then(|quit: &Quit| quit.deadline),
    )
    .await;
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
                deadline: papo_core::deadline(current, QUIT_TIMEOUT),
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
    resolved_sent: &mut VecDeque<String>,
    registration: &mut ConnectWait,
    defaults: &mut PostConnect,
    registered: &mut bool,
    attempt: usize,
    registered_at: &mut Option<Instant>,
    sts: &mut StsActions,
    sts_store: &mut Option<Arc<dyn StsStore>>,
    sts_port: u16,
    sts_websocket: bool,
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
    while let Some(phase) = session.poll_connect_phase() {
        let Some(phase) = crate::ConnectPhase::from_core(phase) else {
            continue;
        };
        enqueue_event(
            events,
            pending_events,
            Event::Connecting(crate::event::Connecting { attempt, phase }),
        );
    }
    drop_closed_waiters(requests, sync_waiters, registration);
    reconcile_pending_events(events, pending_events, requests, resolved_sent);
    let read_ahead = should_read_ahead(requests, sync_waiters, catchups, registration, quitting);
    while (pending_events.is_empty() || read_ahead || after_timeout)
        && let Some(core_event) = session.poll_event()
    {
        match event::from_core_with_receipt(
            core_event,
            session.isupport().casemapping(),
            session.state().nick().origin(),
            session.caps().contains_key(&crate::cap::ACCOUNT_TAG),
            can_prove_non_bot(session),
            crate::rt::now_wall(),
            Some(session.state()),
        ) {
            event::CoreEvent::Forward(event) => {
                catchup::remember_live_event(catchups.values_mut(), &event);
                if let Event::ChannelSynced(synced) = &event {
                    resolve_sync_waiters(session, sync_waiters, &synced.channel);
                }
                if !part_restored_channel(session, defaults, &event)? {
                    update_rejoin_channels(session, defaults, &event);
                }
                let completes_registration = matches!(
                    event,
                    Event::AccountCreation(crate::AccountCreation::VerificationRequired { .. })
                );
                enqueue_event(events, pending_events, event);
                if completes_registration {
                    events.receiver.finish_connecting();
                    registration.succeeded();
                }
            }
            event::CoreEvent::Connected(welcome) => {
                after_connect(session, &welcome, defaults, requests)?;
                *registered_at = Some(now());
                let reconnected = *registered;
                if reconnected {
                    begin_catchup(session, defaults, catchups);
                } else {
                    *registered = true;
                }
                let event = Event::Connected(event::Connected {
                    nick: welcome.nick,
                    welcome: welcome.text,
                    caps: welcome.caps,
                    reconnected,
                });
                enqueue_event(events, pending_events, event);
                events.receiver.finish_connecting();
                registration.succeeded();
            }
            event::CoreEvent::Line(message) => {
                drop_bad_rejoin_key(session, defaults, &message.as_ref());
                let event = crate::event_impl::incoming_from_core(
                    message,
                    session.state().nick().origin(),
                    session.isupport(),
                    session.caps().contains_key(&crate::cap::ACCOUNT_TAG),
                    can_prove_non_bot(session),
                    crate::rt::now_wall(),
                );
                catchup::remember_live_event(catchups.values_mut(), &event);
                if is_resolved_sent_duplicate(resolved_sent, &event) {
                    continue;
                }
                if should_hold_sent_candidate(requests, &event) {
                    hold_event(events, pending_events, event);
                    continue;
                }
                enqueue_event(events, pending_events, event);
            }
            event::CoreEvent::History {
                conversation,
                items,
                end,
            } => {
                if catchups.values().any(|catchup| {
                    matches!(&conversation, crate::Conversation::Channel(channel)
                        if channel == catchup.target.as_str())
                }) {
                    continue;
                }
                let Some(commands) = events.commands.upgrade() else {
                    continue;
                };
                let client = Client {
                    commands,
                    state: Arc::clone(&events.state),
                    origin: session.state().nick().origin(),
                    network_builder: None,
                };
                let items = items
                    .into_iter()
                    .map(|item| {
                        crate::ext::history::history_item(
                            item,
                            Some(&conversation),
                            &client,
                            crate::rt::now_wall(),
                        )
                    })
                    .collect();
                let event = Event::History(crate::HistoryPage {
                    conversation,
                    items,
                    end,
                    reason: crate::HistoryReason::Replay,
                });
                enqueue_event(events, pending_events, event);
            }
            event::CoreEvent::Response(response) => {
                for line in &response.lines {
                    drop_bad_rejoin_key(session, defaults, &line.as_ref());
                }
                if response.error().is_none()
                    && let Some((channel, key)) = join_keys.remove(&response.id)
                    && response_confirms_join(session, &response.lines, &channel)
                {
                    remember_rejoin(session, defaults, Autojoin::new(channel.into_string(), key));
                } else {
                    join_keys.remove(&response.id);
                }
                if let Some(catchup) = catchups.remove(&response.id) {
                    if let Some(event) =
                        advance_catchup(session, events, catchups, defaults, catchup, response)
                    {
                        enqueue_event(events, pending_events, event);
                    }
                } else if let Some(result) = requests.remove(&response.id) {
                    let id = response.id;
                    if let PendingRequest::AutoJoin { channel } = result {
                        if let Some(error) = response.error() {
                            let event = Event::AutoJoinFailed(crate::event::AutoJoinFailed {
                                channel,
                                error: crate::ServerError::from_core(error),
                            });
                            enqueue_event(events, pending_events, event);
                        }
                    } else if let Some(result) = result.respond(response, resolved_sent) {
                        requests.insert(id, result);
                    }
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
                    let disconnected = crate::Disconnected::new(
                        crate::DisconnectCause::connection_lost("transport ended unexpectedly"),
                        None,
                    );
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
            event::CoreEvent::RequestCredentialsRefused(id) => {
                join_keys.remove(&id);
                session.__cancel_request(id);
                if let Some(result) = requests.remove(&id) {
                    result.fail(Error::InvalidInput(InvalidInput::InsecureAuthentication));
                }
            }
            event::CoreEvent::TransmitCredentialsRefused(token) => {
                if let Some(result) = transmits.remove(&token) {
                    result.fail(Error::InvalidInput(InvalidInput::InsecureAuthentication));
                }
            }
            event::CoreEvent::StsUpgrade(port) => sts.upgrade = Some(port),
            event::CoreEvent::StsPolicy(action) => {
                if !cfg!(browser)
                    && let Some(store) = sts_store
                {
                    apply_sts_action(store.as_ref(), sts_port, sts_websocket, &action);
                }
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
        reconcile_pending_events(events, pending_events, requests, resolved_sent);
    }
    drain_transmit(stream, session, pending_write, transmits, deadline).await?;
    Ok(None)
}

const RESOLVED_SENT_CAPACITY: usize = 1_024;

fn remember_resolved_sent(resolved: &mut VecDeque<String>, confirmations: &[papo_core::SentInfo]) {
    for confirmation in confirmations {
        if resolved.len() == RESOLVED_SENT_CAPACITY {
            resolved.pop_front();
        }
        resolved.push_back(confirmation.msgid.clone());
    }
}

fn is_resolved_sent_duplicate(resolved: &mut VecDeque<String>, event: &Event) -> bool {
    let Some(position) =
        own_line_msgid(event).and_then(|msgid| resolved.iter().position(|seen| seen == msgid))
    else {
        return false;
    };
    resolved.remove(position);
    true
}

fn should_hold_sent_candidate(
    requests: &BTreeMap<RequestId, PendingRequest>,
    event: &Event,
) -> bool {
    requests.values().any(PendingRequest::is_text) && own_line_msgid(event).is_some()
}

fn own_line_msgid(event: &Event) -> Option<&str> {
    match event {
        Event::Message(message) if message.authorship == crate::Authorship::Own => {
            message.msgid.as_deref()
        }
        Event::Notice(notice) if notice.authorship == crate::Authorship::Own => {
            notice.msgid.as_deref()
        }
        _ => None,
    }
}

fn reconcile_pending_events(
    events: &EventSink,
    pending: &mut VecDeque<Event>,
    requests: &BTreeMap<RequestId, PendingRequest>,
    resolved_sent: &mut VecDeque<String>,
) {
    let hold = requests.values().any(PendingRequest::is_text);
    let mut retained = VecDeque::new();
    while let Some(event) = pending.pop_front() {
        if is_resolved_sent_duplicate(resolved_sent, &event) {
            continue;
        }
        if hold {
            retained.push_back(event);
        } else {
            send_event(events, event);
        }
    }
    *pending = retained;
}

fn should_read_ahead(
    requests: &BTreeMap<RequestId, PendingRequest>,
    sync_waiters: &[(String, oneshot::Sender<Result<(), Error>>)],
    catchups: &BTreeMap<RequestId, Catchup>,
    registration: &ConnectWait,
    quitting: bool,
) -> bool {
    !requests.is_empty()
        || !sync_waiters.is_empty()
        || !catchups.is_empty()
        || registration.0.is_some()
        || quitting
}

fn text_target(line: &Line) -> Option<&str> {
    let line = line.as_ref();
    matches!(line.command(), ProtoCommand::Privmsg | ProtoCommand::Notice)
        .then(|| line.param(0))
        .flatten()
}

fn can_prove_non_bot(session: &Session) -> bool {
    session.caps().contains_key(&crate::cap::MESSAGE_TAGS)
        && session.isupport().bot_mode().is_some()
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
        let Some(written) = pending.take() else {
            return Ok(true);
        };
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
        let Some(remaining) = pending.buffer.get(pending.offset..) else {
            return Err(io::ErrorKind::InvalidData.into());
        };
        let written = tokio::select! {
            result = stream.write(remaining) => result?,
            () = wait_until(deadline) => return Ok(false),
        };
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        pending.offset += written;
    }
    Ok(true)
}

fn update_rejoin_channels(_session: &Session, defaults: &mut PostConnect, event: &Event) {
    match event {
        Event::Joined(joined) if joined.authorship == crate::Authorship::Own => {
            if !defaults
                .channels
                .iter()
                .any(|entry| joined.channel == entry.channel.as_str())
                && defaults.channels.len() < MAX_REJOIN_CHANNELS
            {
                defaults
                    .channels
                    .push(Autojoin::new(joined.channel.as_ref(), None));
            }
        }
        Event::Parted(parted) if parted.authorship == crate::Authorship::Own => {
            defaults
                .channels
                .retain(|entry| parted.channel != entry.channel.as_str());
            defaults
                .parted_while_disconnected
                .retain(|channel| parted.channel != channel.as_ref());
        }
        Event::Kicked(kicked) if kicked.subject_authorship == crate::Authorship::Own => {
            defaults
                .channels
                .retain(|entry| kicked.channel != entry.channel.as_str());
        }
        Event::ChannelRenamed(renamed) => {
            for entry in &mut defaults.channels {
                if renamed.old == entry.channel.as_str() {
                    entry.channel = renamed.new.to_string();
                }
            }
            for channel in &mut defaults.parted_while_disconnected {
                if renamed.old == channel.as_ref() {
                    *channel = renamed.new.clone();
                }
            }
        }
        _ => {}
    }
}

fn part_restored_channel(
    session: &mut Session,
    defaults: &PostConnect,
    event: &Event,
) -> io::Result<bool> {
    let Event::Joined(joined) = event else {
        return Ok(false);
    };
    if joined.authorship != crate::Authorship::Own
        || !defaults
            .parted_while_disconnected
            .iter()
            .any(|channel| joined.channel == channel.as_ref())
    {
        return Ok(false);
    }
    session
        .send(now(), Line::part(joined.channel.to_string()))
        .map_err(io::Error::other)?;
    Ok(true)
}

fn remember_rejoin(session: &Session, defaults: &mut PostConnect, entry: Autojoin) {
    let casemapping = session.isupport().casemapping();
    defaults
        .channels
        .retain(|configured| casemapping.channel(&configured.channel) != entry.channel.as_str());
    defaults.channels.push(entry);
}

fn response_confirms_join(session: &Session, lines: &[Line], channel: &crate::ChannelName) -> bool {
    let mapping = session.isupport().casemapping();
    lines.iter().any(|line| {
        let line = line.as_ref();
        line.command() == ProtoCommand::Join
            && line.param(0).is_some_and(|target| channel == target)
            && matches!(line.source(), Some(papo_proto::Source::User { nick, .. })
                if mapping.nick(nick) == mapping.nick(session.nick()))
    })
}

fn drop_bad_rejoin_key(
    session: &Session,
    defaults: &mut PostConnect,
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
        .retain(|entry| casemapping.channel(&entry.channel) != channel);
}

fn dedupe_autojoin(session: &Session, defaults: &mut PostConnect) {
    let casemapping = session.isupport().casemapping();
    let mut deduplicated = Vec::with_capacity(defaults.channels.len());
    for entry in defaults.channels.drain(..) {
        deduplicated.retain(|configured: &Autojoin| {
            casemapping.channel(&configured.channel) != entry.channel.as_str()
        });
        deduplicated.push(entry);
    }
    deduplicated.drain(..rejoin_retained_start(deduplicated.len()));
    defaults.channels = deduplicated;
}

fn rejoin_retained_start(channel_count: usize) -> usize {
    channel_count.saturating_sub(MAX_REJOIN_CHANNELS)
}

fn enqueue_event(events: &EventSink, pending: &mut VecDeque<Event>, event: Event) {
    if !pending.is_empty() {
        hold_event(events, pending, event);
        return;
    }
    send_event(events, event);
}

fn hold_event(events: &EventSink, pending: &mut VecDeque<Event>, event: Event) {
    if pending.len() == events.receiver.capacity {
        pending.pop_front();
        events.dropped.fetch_add(1, Ordering::Relaxed);
    }
    pending.push_back(event);
}

fn send_event(events: &EventSink, event: Event) {
    if events.receiver.discard_oldest_if_full() {
        events.dropped.fetch_add(1, Ordering::Relaxed);
    }
    _ = events.regular.try_send(event);
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
            if mapping.channel(&waiting) == channel {
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
    registration: &mut ConnectWait,
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
    let current = Arc::clone(
        &events
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    if current.matches(session, connected) {
        return;
    }
    let next = Arc::new(State::from_session_reusing(session, connected, &current));
    let mut slot = events
        .state
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *slot = next;
}

fn after_connect(
    session: &mut Session,
    welcome: &papo_core::Welcome,
    defaults: &mut PostConnect,
    requests: &mut BTreeMap<RequestId, PendingRequest>,
) -> Result<(), io::Error> {
    dedupe_autojoin(session, defaults);
    if defaults.bot
        && let Some(mode) = welcome.isupport.bot_mode()
    {
        session
            .send(
                now(),
                Line::new("MODE")
                    .with_params([welcome.nick.as_ref().to_owned(), format!("+{mode}")]),
            )
            .map_err(io::Error::other)?;
    }
    if !server_restores_channels(session) {
        for channel in &defaults.channels {
            let mut line = Line::new("JOIN").param(channel.channel.as_str());
            if let Some(key) = &channel.key {
                line = line.param(key);
            }
            let channel_name = session
                .state()
                .nick()
                .origin()
                .__channel(session.isupport().casemapping(), &channel.channel);
            match session.request_with_matcher(
                now(),
                line.clone(),
                ResponseMatcher::Join(channel.channel.clone()),
            ) {
                Ok(id) => {
                    requests.insert(
                        id,
                        PendingRequest::AutoJoin {
                            channel: channel_name,
                        },
                    );
                }
                Err(RequestError::PendingLimit) => {
                    session.send(now(), line).map_err(io::Error::other)?;
                }
                Err(error) => return Err(io::Error::other(error)),
            }
        }
    }
    Ok(())
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

fn disconnected_event(error: &Error, reconnect_in: Option<Duration>) -> crate::Disconnected {
    let cause = match error {
        Error::Disconnected(disconnected) => disconnected.cause.clone(),
        Error::Connect(error) => crate::DisconnectCause::Connect(error.clone()),
        _ => crate::DisconnectCause::connection_lost("transport ended unexpectedly"),
    };
    crate::Disconnected::new(cause, reconnect_in)
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
