use std::{
    collections::{BTreeMap, VecDeque},
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
    Connection, RequestError, RequestId, Response, Session,
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

enum Command {
    Send {
        message: Message,
        result: oneshot::Sender<Result<(), Error>>,
    },
    Request {
        message: Message,
        result: oneshot::Sender<Result<Response, Error>>,
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
}

/// A cheap, cloneable handle to a running IRC client.
#[derive(Clone)]
pub struct Client {
    commands: mpsc::Sender<Command>,
    connected: Arc<AtomicBool>,
    state: Arc<RwLock<Arc<State>>>,
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
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::Request { message, result })
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

    /// Sends a private message to a user or channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the message is invalid or the driver has stopped.
    pub async fn privmsg(&self, target: &str, text: &str) -> Result<(), Error> {
        self.send(Message::privmsg(target, text)).await
    }

    /// Requests information about a nickname.
    ///
    /// # Errors
    ///
    /// Returns an error when the server rejects the request or it cannot complete.
    pub async fn whois(&self, nick: &str) -> Result<crate::Whois, Error> {
        let response = self.raw(Message::new("WHOIS", [nick])).await?;
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
        crate::requests::who(self.raw(message).await?, &isupport, extended)
    }

    /// Joins a channel and waits for its topic and member list to synchronize.
    ///
    /// # Errors
    ///
    /// Returns an error when the server rejects the join or it cannot complete.
    pub async fn join(&self, channel: &str) -> Result<crate::ChannelInfo, Error> {
        let response = self.raw(Message::join(channel)).await?;
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
        crate::requests::empty(&self.raw(Message::new("PART", params)).await?)
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
        crate::requests::empty(&self.raw(Message::new("MODE", params)).await?)
    }

    /// Sets a channel topic, or queries it when `text` is `None`.
    ///
    /// # Errors
    ///
    /// Returns an error when the server rejects the request or it cannot complete.
    pub async fn topic(&self, channel: &str, text: Option<&str>) -> Result<Option<String>, Error> {
        let mut params = vec![channel];
        params.extend(text);
        crate::requests::topic(&self.raw(Message::new("TOPIC", params)).await?)
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
    if is_fatal_failure(&end.error, &end.reason) {
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
    if sts_required || is_fatal_failure(&error, &reason) || !reconnect.permits(attempts) {
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
        Some(Command::Request { result, .. }) => {
            _ = result.send(Err(Error::Disconnected));
            true
        }
        Some(Command::WaitChannelSync { result, .. }) => {
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
    let mut sync_waiters = Vec::new();
    let mut registered_at = None;
    let mut sts = StsActions::default();
    let mut buffer = [0; 4096];
    let (mut reason, mut error, quit_timed_out) = loop {
        match drain(
            &mut stream,
            session,
            events,
            &mut pending_event,
            &mut requests,
            &mut sync_waiters,
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
                Some(Command::Request { message, result }) => {
                    if !session.caps().contains_key("labeled-response") {
                        _ = result.send(Err(Error::Unsupported("labeled-response")));
                        continue;
                    }
                    match session.request(now(), message) {
                        Ok(id) => {
                            requests.insert(id, result);
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
                        true,
                    );
                }
                session.handle_timeout(current);
            },
        }
    };
    session.handle_disconnected(now());
    for (_, result) in requests {
        _ = result.send(Err(Error::Disconnected));
    }
    for (_, result) in sync_waiters {
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

#[allow(clippy::too_many_arguments)]
async fn drain<S>(
    stream: &mut S,
    session: &mut Session,
    events: &EventSink,
    pending_event: &mut Option<Event>,
    requests: &mut BTreeMap<RequestId, oneshot::Sender<Result<Response, Error>>>,
    sync_waiters: &mut Vec<(String, oneshot::Sender<Result<(), Error>>)>,
    registration: &mut RegistrationWait,
    defaults: &PostRegistration,
    registered: &mut bool,
    registered_at: &mut Option<Instant>,
    sts: &mut StsActions,
    sts_store: &mut Option<Box<dyn StsStore + Send>>,
    sts_port: u16,
) -> io::Result<Option<String>>
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
                        if let Some(result) = requests.remove(&response.id) {
                            _ = result.send(Ok(response));
                            continue;
                        }
                        Event::Response(response)
                    }
                    Event::RequestTimedOut { id } => {
                        if let Some(result) = requests.remove(&id) {
                            _ = result.send(Err(Error::Timeout));
                            continue;
                        }
                        Event::RequestTimedOut { id }
                    }
                    Event::RequestAborted { id } => {
                        if let Some(result) = requests.remove(&id) {
                            _ = result.send(Err(Error::Disconnected));
                            continue;
                        }
                        Event::RequestAborted { id }
                    }
                    event => event,
                };
                if let Event::ChannelSynced { channel } = &event {
                    resolve_sync_waiters(session, sync_waiters, channel);
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
            event::CoreEvent::Close(reason) => return Ok(Some(reason)),
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
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use papo_core::{
        Config, Decoding, Sasl,
        sts::{ConnectionSecurity, StsAction},
    };
    use papo_proto::Message;
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
        client_line(&mut server, b"@label=7 WHOIS missing-nick\r\n").await;
        server_line(
            &mut server,
            b"@label=7 :ergo.test 401 papo-driver missing-nick :No such nick\r\n",
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
        client_line(&mut server, b"@label=8 WHOIS alice\r\n").await;
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(matches!(timed_out.await.unwrap(), Err(Error::Timeout)));

        let aborted =
            tokio::spawn(async move { client.raw(Message::new("WHOIS", ["emily"])).await });
        client_line(&mut server, b"@label=9 WHOIS emily\r\n").await;
        server.shutdown().await.unwrap();
        assert!(matches!(aborted.await.unwrap(), Err(Error::Disconnected)));
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
            ReplyKind::Reply => message.reply("pong").await.unwrap(),
            #[cfg(feature = "bot")]
            ReplyKind::PrivateNotice => message.private_notice("denied").await.unwrap(),
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
}
