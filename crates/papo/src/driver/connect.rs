use std::{collections::VecDeque, ops::ControlFlow};

use papo_core::{
    Connection, Session,
    sts::{ConnectionSecurity, StoredPolicy, StsAction, StsStore, reschedule_on_disconnect},
};
use papo_proto::Message;
use tokio::sync::mpsc;

use super::{
    BackoffEntropy, BoxConnection, Command, ConnectFuture, ConnectionEnd, ConnectionPlan,
    Connector, EventSink, PostRegistration, RegistrationWait, SLOW_CONSUMER_REASON,
    STABLE_CONNECTION, StsActions, enqueue_event, finish, now, run_connection,
};
use crate::{AddressError, Error, Event, Reconnect, address::Protocol, transport};

pub(super) async fn open_connection(plan: ConnectionPlan) -> Result<transport::Stream, Error> {
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

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) async fn run_plan(
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
    let mut pending_events = VecDeque::new();
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
            &events,
            &mut pending_events,
            sts_required,
            reconnect,
            attempts,
            entropy.as_mut(),
        )
        .await
        {
            ConnectionAttempt::Connected(stream) => stream,
            ConnectionAttempt::Retry => continue,
            ConnectionAttempt::Stop => {
                events.finish(pending_events);
                return Ok(());
            }
            ConnectionAttempt::SlowConsumer => {
                return finish_slow_consumer(events, &mut registration, pending_events);
            }
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
            std::mem::take(&mut pending_events),
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
        pending_events = match enqueue_retry_event(&events, end) {
            Ok(pending) => pending,
            Err(end) => return finish(events, *end, false),
        };
        if upgrade_port.is_some() {
            continue;
        }
        if !wait_before_retry(
            reconnect,
            attempts,
            &mut commands,
            &mut queued,
            disconnected_queue_capacity,
            &events.regular,
            &mut pending_events,
            entropy.as_mut(),
        )
        .await
        {
            events.finish(pending_events);
            return Ok(());
        }
    }
}

fn enqueue_retry_event(
    events: &EventSink,
    mut end: ConnectionEnd,
) -> Result<VecDeque<Event>, Box<ConnectionEnd>> {
    let reason = std::mem::take(&mut end.reason);
    if enqueue_event(
        events,
        &mut end.pending_events,
        Event::Disconnected {
            reason,
            will_retry: true,
        },
    ) {
        return Ok(end.pending_events);
    }
    SLOW_CONSUMER_REASON.clone_into(&mut end.reason);
    end.error = Error::Disconnected;
    Err(Box::new(end))
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
pub(super) async fn run_planned_connection(
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
    pending_events: VecDeque<Event>,
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
        pending_events,
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
    if end.reason == SLOW_CONSUMER_REASON {
        return ControlFlow::Break((end, true));
    }
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
    SlowConsumer,
}

#[allow(clippy::too_many_arguments)]
async fn connect_or_wait(
    connection: ConnectFuture,
    commands: &mut mpsc::Receiver<Command>,
    queued: &mut VecDeque<Message>,
    capacity: usize,
    events: &EventSink,
    pending_events: &mut VecDeque<Event>,
    sts_required: bool,
    reconnect: Reconnect,
    attempts: usize,
    entropy: &mut (dyn FnMut() -> u64 + Send),
) -> ConnectionAttempt {
    let Some(result) = connect_while_disconnected(
        connection,
        commands,
        queued,
        capacity,
        &events.regular,
        pending_events,
    )
    .await
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
    if !enqueue_event(
        events,
        pending_events,
        Event::Disconnected {
            reason,
            will_retry: true,
        },
    ) {
        return ConnectionAttempt::SlowConsumer;
    }
    if wait_before_retry(
        reconnect,
        attempts,
        commands,
        queued,
        capacity,
        &events.regular,
        pending_events,
        entropy,
    )
    .await
    {
        ConnectionAttempt::Retry
    } else {
        ConnectionAttempt::Stop
    }
}

pub(super) fn record_sts_upgrade(last_host: &mut Option<String>, host: &str) -> Result<(), Error> {
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
        events.finish(VecDeque::from([Event::Disconnected {
            reason,
            will_retry: false,
        }]));
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
            pending_events: VecDeque::new(),
        },
        false,
    )
}

fn finish_slow_consumer(
    events: EventSink,
    registration: &mut RegistrationWait,
    pending_events: VecDeque<Event>,
) -> Result<(), Error> {
    registration.failed(Error::Registration(SLOW_CONSUMER_REASON.to_owned()));
    finish(
        events,
        ConnectionEnd {
            reason: SLOW_CONSUMER_REASON.to_owned(),
            error: Error::Disconnected,
            close_kind: None,
            quit: None,
            quit_timed_out: false,
            registered_at: None,
            sts: StsActions::default(),
            sts_store: None,
            pending_events,
        },
        false,
    )
}

pub(super) fn apply_sts_action(store: &mut dyn StsStore, port: u16, action: &StsAction) {
    match action {
        StsAction::Persist { host, duration } => {
            let now = crate::rt::now_wall();
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
        store.put(reschedule_on_disconnect(policy, crate::rt::now_wall()));
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
    if !policy.is_active_at(crate::rt::now_wall()) {
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

pub(super) fn is_fatal_failure(error: &Error, close_kind: Option<papo_core::CloseKind>) -> bool {
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
    events: &mpsc::Sender<Event>,
    pending_events: &mut VecDeque<Event>,
) -> Option<Result<BoxConnection, Error>> {
    loop {
        tokio::select! {
            result = &mut connection => return Some(result),
            permit = events.reserve(), if !pending_events.is_empty() => match permit {
                Ok(permit) => permit.send(pending_events.pop_front().expect("pending event is present")),
                Err(_) => pending_events.clear(),
            },
            command = commands.recv() => {
                if !handle_disconnected_command(command, queued, capacity) {
                    return None;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn wait_before_retry(
    reconnect: Reconnect,
    attempts: usize,
    commands: &mut mpsc::Receiver<Command>,
    queued: &mut VecDeque<Message>,
    capacity: usize,
    events: &mpsc::Sender<Event>,
    pending_events: &mut VecDeque<Event>,
    entropy: &mut (dyn FnMut() -> u64 + Send),
) -> bool {
    let retry = u32::try_from(attempts.saturating_sub(1)).unwrap_or(u32::MAX);
    let sleep = crate::rt::sleep_until(now() + reconnect.delay(retry, entropy()));
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            () = &mut sleep => return true,
            permit = events.reserve(), if !pending_events.is_empty() => match permit {
                Ok(permit) => permit.send(pending_events.pop_front().expect("pending event is present")),
                Err(_) => pending_events.clear(),
            },
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
