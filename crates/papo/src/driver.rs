use std::{
    collections::VecDeque,
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task,
    time::{Instant, SystemTime},
};

use papo_core::{
    Connection, Session,
    sts::{ConnectionSecurity, StoredPolicy, StsAction, StsStore, reschedule_on_disconnect},
};
use papo_proto::Message;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

use crate::{
    AddressError, Error, Event, Reconnect,
    address::{Protocol, ServerAddress},
    event,
    transport::{self, TlsConfig},
};

pub(crate) const DEFAULT_CHANNEL_CAPACITY: usize = 1024;
const QUIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub(crate) trait ConnectionIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ConnectionIo for T {}
type BoxConnection = Box<dyn ConnectionIo>;
type ConnectFuture = Pin<Box<dyn Future<Output = Result<BoxConnection, Error>> + Send>>;
pub(crate) type Connector = Arc<dyn Fn(ConnectionPlan) -> ConnectFuture + Send + Sync>;

enum Command {
    Send {
        message: Message,
        result: oneshot::Sender<Result<(), Error>>,
    },
    Quit {
        reason: Option<String>,
        result: oneshot::Sender<Result<(), Error>>,
    },
}

struct Quit {
    deadline: Instant,
    results: Vec<oneshot::Sender<Result<(), Error>>>,
}

struct EventSink {
    regular: mpsc::Sender<Event>,
    disconnected: oneshot::Sender<Event>,
    connected: Arc<AtomicBool>,
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
}

/// A cheap, cloneable handle to a running IRC client.
#[derive(Clone)]
pub struct Client {
    commands: mpsc::Sender<Command>,
    connected: Arc<AtomicBool>,
}

impl Client {
    /// Starts configuring a client for `address`.
    pub fn builder(address: &str) -> crate::Builder {
        crate::Builder::new(address)
    }

    /// Reports whether an IRC transport is currently connected.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
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

    /// Sends a private message to a user or channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the message is invalid or the driver has stopped.
    pub async fn privmsg(&self, target: &str, text: &str) -> Result<(), Error> {
        self.send(Message::privmsg(target, text)).await
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
        let channels = channels(capacity);
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
        let channels = channels(capacity);
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
        ));
        (channels.client, channels.events, Self { inner })
    }
}

fn channels(capacity: usize) -> Channels {
    let (command_sender, command_receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
    let (event_sender, event_receiver) = mpsc::channel(capacity);
    let (disconnect_sender, disconnect_receiver) = oneshot::channel();
    let connected = Arc::new(AtomicBool::new(false));
    let events = EventSink {
        regular: event_sender,
        disconnected: disconnect_sender,
        connected: Arc::clone(&connected),
    };
    Channels {
        client: Client {
            commands: command_sender,
            connected,
        },
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

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
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
) -> Result<(), Error> {
    let mut base_plan = match plan {
        Ok(plan) => plan,
        Err(error) => {
            if registration.0.is_some() {
                registration.failed(Error::Address(error));
                return Ok(());
            }
            return Err(Error::Address(error));
        }
    };
    let mut attempts = 0_usize;
    let mut registered = false;
    let mut queued = VecDeque::new();
    let mut sts_required = false;
    loop {
        attempts = attempts.saturating_add(1);
        let plan = match plan_with_stored_sts(&base_plan, sts_store.as_mut(), force_plaintext) {
            Ok(plan) => plan,
            Err(error) => {
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
                return finish(
                    events,
                    ConnectionEnd {
                        reason,
                        error,
                        quit: None,
                        quit_timed_out: false,
                        registered_at: None,
                        sts: StsActions::default(),
                    },
                    false,
                );
            }
        };
        sts_required |= !base_plan.address.is_secure() && plan.address.is_secure();
        let Some(connection_result) = connect_while_disconnected(
            connector(plan.clone()),
            &mut commands,
            &mut queued,
            disconnected_queue_capacity,
        )
        .await
        else {
            return Ok(());
        };
        let stream = match connection_result {
            Ok(stream) => stream,
            Err(error) => {
                let reason = error.to_string();
                if sts_required || is_fatal_failure(&error, &reason) || !reconnect.permits(attempts)
                {
                    if registration.0.is_some() {
                        registration.failed(error);
                        drop(events.regular);
                        _ = events.disconnected.send(Event::Disconnected {
                            reason,
                            will_retry: false,
                        });
                        return Ok(());
                    }
                    return finish(
                        events,
                        ConnectionEnd {
                            reason,
                            error,
                            quit: None,
                            quit_timed_out: false,
                            registered_at: None,
                            sts: StsActions::default(),
                        },
                        false,
                    );
                }
                _ = events
                    .regular
                    .send(Event::Disconnected {
                        reason,
                        will_retry: true,
                    })
                    .await;
                if !wait_before_retry(
                    reconnect,
                    attempts,
                    &mut commands,
                    &mut queued,
                    disconnected_queue_capacity,
                )
                .await
                {
                    return Ok(());
                }
                continue;
            }
        };
        let security = if plan.address.is_secure() {
            ConnectionSecurity::Secure
        } else {
            ConnectionSecurity::Plaintext
        };
        let mut end = run_connection(
            stream,
            &mut session,
            &mut commands,
            &events.regular,
            &mut registration,
            &defaults,
            &mut queued,
            &mut registered,
            Some(Connection {
                host: plan.address.host.clone(),
                security,
            }),
            &events.connected,
        )
        .await;
        apply_sts_actions(sts_store.as_mut(), &plan, &end.sts);
        reschedule_stored_policy(sts_store.as_mut(), &plan.address.host);
        if end.quit.is_some() {
            return finish(events, end, false);
        }
        if let Some(port) = end.sts.upgrade {
            if force_plaintext {
                end.reason = Error::StsRequiresTls {
                    host: plan.address.host.clone(),
                }
                .to_string();
                end.error = Error::StsRequiresTls {
                    host: plan.address.host.clone(),
                };
                registration.failed(end.registration_error());
                return finish(events, end, false);
            }
            base_plan.address.set_tls(true);
            base_plan.address.set_port(port);
            sts_required = true;
            _ = events
                .regular
                .send(Event::Disconnected {
                    reason: end.reason,
                    will_retry: true,
                })
                .await;
            continue;
        }
        if is_fatal_failure(&end.error, &end.reason) {
            registration.failed(end.registration_error());
            return finish(events, end, false);
        }
        if !reconnect.permits(attempts) {
            registration.failed(end.registration_error());
            return finish(events, end, false);
        }
        _ = events
            .regular
            .send(Event::Disconnected {
                reason: end.reason,
                will_retry: true,
            })
            .await;
        if !wait_before_retry(
            reconnect,
            attempts,
            &mut commands,
            &mut queued,
            disconnected_queue_capacity,
        )
        .await
        {
            return Ok(());
        }
    }
}

fn apply_sts_actions(store: &mut dyn StsStore, plan: &ConnectionPlan, actions: &StsActions) {
    for action in &actions.policies {
        match action {
            StsAction::Persist { host, duration } => {
                let now = SystemTime::now();
                if let Some(expires_at) = now.checked_add(*duration) {
                    store.put(StoredPolicy {
                        host: host.clone(),
                        port: plan.address.port,
                        expires_at,
                        duration: *duration,
                    });
                }
            }
            StsAction::Remove { host } => store.remove(host),
            _ => {}
        }
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

fn is_fatal_failure(error: &Error, reason: &str) -> bool {
    if reason.starts_with("SASL failed:") {
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
) -> bool {
    let retry = u32::try_from(attempts.saturating_sub(1)).unwrap_or(u32::MAX);
    let sleep = tokio::time::sleep(reconnect.delay(retry, crate::OsEntropy::next_u64()));
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
        &events.regular,
        &mut registration,
        &defaults,
        &mut queued,
        &mut registered,
        None,
        &events.connected,
    )
    .await;
    registration.failed(end.registration_error());
    finish(events, end, false)
}

struct ConnectionEnd {
    reason: String,
    error: Error,
    quit: Option<Quit>,
    quit_timed_out: bool,
    #[allow(dead_code)]
    registered_at: Option<Instant>,
    sts: StsActions,
}

#[derive(Default)]
struct StsActions {
    upgrade: Option<u16>,
    policies: Vec<StsAction>,
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
    events: &mpsc::Sender<Event>,
    registration: &mut RegistrationWait,
    defaults: &PostRegistration,
    queued: &mut VecDeque<Message>,
    registered: &mut bool,
    connection: Option<Connection>,
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
    while let Some(message) = queued.pop_front() {
        session
            .send(now(), message)
            .expect("disconnected commands were validated before queueing");
    }
    let mut commands_open = true;
    let mut quit = None;
    let mut pending_event = None;
    let mut registered_at = None;
    let mut sts = StsActions::default();
    let mut buffer = [0; 4096];
    let (mut reason, mut error, quit_timed_out) = loop {
        match drain(
            &mut stream,
            session,
            &mut pending_event,
            registration,
            defaults,
            registered,
            &mut registered_at,
            &mut sts,
        )
        .await
        {
            Ok(Some(reason)) => break (reason, Error::Disconnected, false),
            Ok(None) => {}
            Err(error) => break (error.to_string(), Error::Io(error), false),
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
                    break ("server closed the connection".to_owned(), Error::Disconnected, false);
                }
                Ok(count) => session.handle_bytes(now(), &buffer[..count]),
                Err(error) => break (error.to_string(), Error::Io(error), false),
            },
            permit = events.clone().reserve_owned(), if pending_event.is_some() => match permit {
                Ok(permit) => {
                    _ = permit.send(pending_event.take().expect("pending event is present"));
                }
                Err(_) => pending_event = None,
            },
            command = commands.recv(), if commands_open => match command {
                Some(Command::Send { message, result }) => {
                    _ = result.send(session.send(now(), message).map_err(Error::Build));
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
                        true,
                    );
                }
                session.handle_timeout(current);
            },
        }
    };
    session.handle_disconnected(now());
    connected.store(false, Ordering::Relaxed);
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
        quit,
        quit_timed_out,
        registered_at,
        sts,
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

#[allow(clippy::too_many_arguments)]
async fn drain<S>(
    stream: &mut S,
    session: &mut Session,
    pending_event: &mut Option<Event>,
    registration: &mut RegistrationWait,
    defaults: &PostRegistration,
    registered: &mut bool,
    registered_at: &mut Option<Instant>,
    sts: &mut StsActions,
) -> io::Result<Option<String>>
where
    S: AsyncWrite + Unpin,
{
    while let Some(line) = session.poll_transmit() {
        stream.write_all(&line).await?;
    }
    while pending_event.is_none()
        && let Some(core_event) = session.poll_event()
    {
        match event::from_core(core_event) {
            event::CoreEvent::Forward(mut event) => {
                match &event {
                    Event::StsUpgrade { port } => sts.upgrade = Some(*port),
                    Event::StsPolicy(action) => sts.policies.push(action.clone()),
                    _ => {}
                }
                if let Event::Registered(welcome) = &event {
                    after_registration(session, welcome, defaults)?;
                    registration.succeeded();
                    *registered_at = Some(now());
                    if *registered {
                        event = Event::Reconnected(welcome.clone());
                    } else {
                        *registered = true;
                    }
                }
                *pending_event = Some(event);
            }
            event::CoreEvent::Close(reason) => return Ok(Some(reason)),
            event::CoreEvent::Unknown => {}
        }
    }
    Ok(None)
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
    if should_rejoin(welcome) {
        for channel in &defaults.channels {
            session
                .send(now(), Message::new("JOIN", [channel.clone()]))
                .map_err(io::Error::other)?;
        }
    }
    Ok(())
}

fn should_rejoin(_welcome: &papo_core::Welcome) -> bool {
    // Core does not yet expose whether draft/persistence restored channel membership.
    true
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
        time::Duration,
    };

    use papo_core::{
        Config, Decoding, Sasl,
        sts::{ConnectionSecurity, StsAction},
    };
    use papo_proto::Message;
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};

    use super::{ConnectionPlan, Driver, Error, Event, Session};
    use crate::{Reconnect, transport::TlsConfig};

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

    async fn complete_registration(server: &mut DuplexStream, events: &mut super::Events) {
        registration_burst(server).await;
        server_line(server, b":ergo.test CAP * LS :\r\n").await;
        client_line(server, b"CAP END\r\n").await;
        server_line(server, b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
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
        let (_client, mut events, driver) = Driver::with_connector(
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
            Session::new(config()),
            super::DEFAULT_CHANNEL_CAPACITY,
            None,
            super::PostRegistration {
                bot: false,
                channels: vec!["#papo".into()],
            },
            Reconnect::exponential(),
            256,
            Box::new(papo_core::sts::MemoryStsStore::new()),
            false,
            connector,
        );
        let driver = tokio::spawn(driver);

        complete_registration(&mut first_server, &mut events).await;
        client_line(&mut first_server, b"JOIN #papo\r\n").await;
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
        server_line(&mut second_server, b":ergo.test CAP * LS :\r\n").await;
        client_line(&mut second_server, b"CAP END\r\n").await;
        server_line(
            &mut second_server,
            b":ergo.test 001 papo-driver :Welcome back\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
        )
        .await;
        assert!(matches!(events.next().await, Some(Event::Reconnected(_))));
        client_line(&mut second_server, b"JOIN #papo\r\n").await;
        assert!(client.is_connected());

        let quit = tokio::spawn(async move { client.quit(None).await });
        client_line(&mut second_server, b"QUIT\r\n").await;
        second_server.shutdown().await.unwrap();
        assert!(quit.await.unwrap().is_ok());
        assert!(driver.await.unwrap().is_ok());
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
            1,
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
        assert!(matches!(
            client.privmsg("#papo", "queue overflow").await,
            Err(Error::Disconnected)
        ));

        tokio::time::advance(Duration::from_secs(2)).await;
        registration_burst(&mut server).await;
        client_line(&mut server, b"WHOIS emily\r\n").await;
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

    #[tokio::test(flavor = "current_thread")]
    async fn eof_emits_disconnected_and_returns_an_error() {
        let (mut server, _client, mut events, driver) = scripted(config());
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server.shutdown().await.unwrap();
        assert_eq!(
            events.next().await,
            Some(Event::Disconnected {
                reason: "server closed the connection".into(),
                will_retry: false,
            })
        );
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_error_overrides_a_non_io_terminal_cause() {
        let (mut server, _client, mut events, driver) = scripted_with_failures(false);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server.shutdown().await.unwrap();
        assert_eq!(
            events.next().await,
            Some(Event::Disconnected {
                reason: "shutdown failed".into(),
                will_retry: false,
            })
        );
        assert!(matches!(driver.await.unwrap(), Err(Error::Io(error))
            if error.to_string() == "shutdown failed"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_error_does_not_replace_an_io_terminal_cause() {
        let (mut server, _client, mut events, driver) = scripted_with_failures(true);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        assert_eq!(
            events.next().await,
            Some(Event::Disconnected {
                reason: "original read failure".into(),
                will_retry: false,
            })
        );
        assert!(matches!(driver.await.unwrap(), Err(Error::Io(error))
            if error.to_string() == "original read failure"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn overlong_line_becomes_a_terminal_disconnect() {
        let (mut server, _client, mut events, driver) = scripted(config());
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server.write_all(&vec![b'x'; 8704]).await.unwrap();
        assert_eq!(
            events.next().await,
            Some(Event::Disconnected {
                reason: "incoming IRC line exceeds 8703 bytes".into(),
                will_retry: false,
            })
        );
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
        assert_eq!(
            events.next().await,
            Some(Event::Malformed {
                line: b":ergo.test NOTICE * :caf\xe9".to_vec()
            })
        );
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
        assert!(super::is_fatal_failure(&error, &error.to_string()));
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
        assert_eq!(events.next().await, Some(Event::StsUpgrade { port: 6697 }));
        assert_eq!(
            events.next().await,
            Some(Event::Disconnected {
                reason: "sts upgrade".into(),
                will_retry: false,
            })
        );
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
        assert_eq!(events.next().await, Some(Event::StsUpgrade { port: 6699 }));
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
        client_line(&mut server, b"CAP END\r\n").await;
        server.shutdown().await.unwrap();
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));

        let policy = papo_core::sts::StsStore::get(&inspection, "irc.example.net").unwrap();
        assert_eq!(policy.port, 7443);
        assert_eq!(policy.duration, Duration::from_secs(3600));
        assert!(policy.expires_at > std::time::SystemTime::now());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_send_reaches_the_wire() {
        let (mut server, client, _events, driver) = scripted(config());
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        client
            .send(papo_proto::Message::new("WHOIS", ["emily"]))
            .await
            .unwrap();
        let mut whois = [0; 13];
        server.read_exact(&mut whois).await.unwrap();
        assert_eq!(&whois, b"WHOIS emily\r\n");
        server.shutdown().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
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
        assert_eq!(
            events.next().await,
            Some(Event::Disconnected {
                reason: "original read failure".into(),
                will_retry: false,
            })
        );
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
        assert_eq!(
            events.next().await,
            Some(Event::Disconnected {
                reason: "QUIT was not acknowledged within 10 seconds".into(),
                will_retry: false,
            })
        );
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
}
