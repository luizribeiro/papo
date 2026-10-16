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
use papo_proto::{Message, Typing};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

use crate::{
    AddressError, Error, Event, Reconnect,
    address::{Protocol, ServerAddress},
    client::{Client, Sent},
    event,
    state::State,
    transport::{self, TlsConfig},
};

#[cfg(test)]
use crate::client::{AccountRegistration, PushKeys};

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
    Rename {
        old: String,
        new: String,
        reason: String,
    },
    Relay {
        channel: String,
        nick: String,
        text: String,
    },
    WebPushRegister {
        endpoint: String,
        p256dh: String,
        auth: String,
    },
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
            Self::PersistenceGet => session.persistence_get(now())?,
            Self::PersistenceSet(setting) => session.persistence_set(now(), setting)?,
            Self::Metadata(command) => return session.metadata(now(), command),
            Self::Rename { old, new, reason } => return session.rename(now(), &old, &new, &reason),
            Self::Relay {
                channel,
                nick,
                text,
            } => return session.relay(now(), &channel, &nick, &text),
            Self::WebPushRegister {
                endpoint,
                p256dh,
                auth,
            } => return session.webpush_register(now(), &endpoint, &p256dh, &auth),
            Self::WebPushUnregister(endpoint) => {
                return session.webpush_unregister(now(), &endpoint);
            }
        }
        Ok(before)
    }
}

struct Catchup {
    target: String,
    messages: Vec<Message>,
    seen: HashSet<String>,
}

pub(crate) enum Command {
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
    pub(crate) tls: TlsConfig,
}

#[derive(Default)]
pub(crate) struct PostRegistration {
    pub(crate) bot: bool,
    pub(crate) channels: Vec<String>,
    pub(crate) history_on_reconnect: bool,
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

    pub(crate) fn with_stream_and_registration<S>(
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

pub(crate) fn channels(capacity: usize, session: &Session) -> Channels {
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
        Some(
            papo_core::CloseKind::SaslFailed
                | papo_core::CloseKind::AccountRequired
                | papo_core::CloseKind::WebIrcRejected
        )
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
pub(crate) mod test_support;

#[cfg(test)]
mod tests;
