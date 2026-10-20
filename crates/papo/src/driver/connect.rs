use std::{collections::VecDeque, ops::ControlFlow, time::Duration};

#[cfg(not(browser))]
use papo_core::sts::{StsAction, StsPolicy};
use papo_core::{
    Connection, Session,
    sts::{ConnectionSecurity, StsStore, reschedule_on_disconnect},
};
use tokio::sync::mpsc;

use super::{
    BackoffEntropy, BoxConnection, Command, ConnectFuture, ConnectWait, ConnectionEnd,
    ConnectionPlan, Connector, EventSink, PostConnect, SLOW_CONSUMER_REASON, STABLE_CONNECTION,
    StsActions, enqueue_event, finish, now, run_connection,
};
#[cfg(sockets)]
use crate::address::Protocol;
use crate::{Error, Event, InvalidInput, Reconnect, transport};

pub(super) async fn open_connection(plan: ConnectionPlan) -> Result<transport::Stream, Error> {
    #[cfg(sockets)]
    {
        if let Some(url) = plan.address.websocket_url() {
            #[cfg(feature = "websocket")]
            {
                return transport::connect_websocket(
                    &url,
                    plan.address.is_secure().then_some(&plan.tls),
                )
                .await
                .map_err(|error| Error::Connect(error.into()));
            }
            #[cfg(not(feature = "websocket"))]
            {
                _ = url;
                return Err(Error::connect_unavailable("websocket"));
            }
        }
        let security = match plan.address.protocol {
            Protocol::Irc => transport::Security::Plaintext,
            Protocol::Ircs => transport::Security::Tls(plan.tls),
            Protocol::Ws | Protocol::Wss => return Err(Error::connect_unavailable("websocket")),
        };
        transport::connect(&plan.address.host, plan.address.port, &security)
            .await
            .map_err(|error| Error::Connect(error.into()))
    }
    #[cfg(browser)]
    {
        let Some(url) = plan.address.websocket_url() else {
            return Err(Error::connect_unavailable("TCP in browsers"));
        };
        transport::connect_websocket(&url, None)
            .await
            .map_err(|error| Error::Connect(error.into()))
    }
    #[cfg(all(not(browser), not(sockets)))]
    {
        _ = plan;
        Err(Error::connect_unavailable("outbound sockets"))
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) async fn run_plan(
    plan: Result<ConnectionPlan, InvalidInput>,
    mut session: Session,
    mut commands: mpsc::Receiver<Command>,
    events: EventSink,
    mut registration: ConnectWait,
    mut defaults: PostConnect,
    reconnect: Reconnect,
    mut sts_store: Box<dyn StsStore + Send>,
    connector: Connector,
    mut entropy: BackoffEntropy,
) -> Result<(), Error> {
    let Some(mut base_plan) = initial_connection_plan(plan, &mut registration)? else {
        return Ok(());
    };
    let mut attempts = 0_usize;
    let mut registered = false;
    let mut pending_events = VecDeque::new();
    let mut sts_required = false;
    let mut last_sts_upgrade = None;
    let mut last_disconnect = None;
    loop {
        attempts = attempts.saturating_add(1);
        let plan = plan_with_stored_sts(&base_plan, sts_store.as_mut());
        sts_required |= !base_plan.address.is_secure() && plan.address.is_secure();
        let stream = match connect_or_wait(
            connector(plan.clone()),
            &mut commands,
            &events,
            &mut pending_events,
            registration.0.is_some(),
            sts_required,
            reconnect,
            attempts,
            entropy.as_mut(),
            &mut last_disconnect,
        )
        .await
        {
            ConnectionAttempt::Connected(stream) => stream,
            ConnectionAttempt::Retry => continue,
            ConnectionAttempt::Stop => {
                finish_requested(events, pending_events);
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
            &mut defaults,
            &mut registered,
            &plan,
            sts_store,
            std::mem::take(&mut pending_events),
        )
        .await;
        if end.registered_at.is_some() {
            defaults.disconnected_at = Some(crate::rt::now_wall());
        }
        sts_store = end
            .sts_store
            .take()
            .expect("planned connections retain their STS store");
        reschedule_stored_policy(sts_store.as_mut(), &plan.address.host);
        let (mut end, upgrade_port) = match triage_connection_end(
            end,
            &plan,
            registration.0.is_some(),
            reconnect,
            &mut attempts,
            &mut last_sts_upgrade,
        ) {
            ControlFlow::Break((mut end, login_failed)) => {
                if login_failed {
                    let error = end.login_error();
                    registration.failed(error.clone());
                    end.error = error;
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
        let reconnect_now = end.reconnect_now;
        let retry_in = if upgrade_port.is_some() || reconnect_now {
            Duration::ZERO
        } else {
            retry_delay(reconnect, attempts, entropy.as_mut())
        };
        let disconnected = super::disconnected_event(&end.error, Some(retry_in));
        if matches!(disconnected.cause, crate::DisconnectCause::SlowConsumer) {
            end.pending_events.clear();
        }
        last_disconnect = Some(disconnected.cause.clone());
        pending_events = match enqueue_retry_event(&events, end, disconnected.clone()) {
            Ok(pending) => pending,
            Err(end) => return finish(events, *end, false),
        };
        if upgrade_port.is_some() || reconnect_now {
            continue;
        }
        if !wait_before_retry(
            &mut commands,
            &events.regular,
            &mut pending_events,
            disconnected,
        )
        .await
        {
            finish_requested(events, pending_events);
            return Ok(());
        }
    }
}

fn finish_requested(events: EventSink, mut pending_events: VecDeque<Event>) {
    pending_events.push_back(Event::Disconnected(crate::Disconnected::new(
        crate::DisconnectCause::Requested {
            quit_unacknowledged: false,
        },
        None,
    )));
    events.finish(pending_events);
}

fn enqueue_retry_event(
    events: &EventSink,
    mut end: ConnectionEnd,
    disconnected: crate::Disconnected,
) -> Result<VecDeque<Event>, Box<ConnectionEnd>> {
    if enqueue_event(
        events,
        &mut end.pending_events,
        Event::Disconnected(disconnected),
    ) {
        return Ok(end.pending_events);
    }
    SLOW_CONSUMER_REASON.clone_into(&mut end.reason);
    end.error = Error::disconnected_slow_consumer();
    Err(Box::new(end))
}

fn initial_connection_plan(
    plan: Result<ConnectionPlan, InvalidInput>,
    registration: &mut ConnectWait,
) -> Result<Option<ConnectionPlan>, Error> {
    match plan {
        Ok(plan) => Ok(Some(plan)),
        Err(error) if registration.0.is_some() => {
            registration.failed(Error::InvalidInput(error));
            Ok(None)
        }
        Err(error) => Err(Error::InvalidInput(error)),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_planned_connection(
    stream: BoxConnection,
    session: &mut Session,
    commands: &mut mpsc::Receiver<Command>,
    events: &EventSink,
    registration: &mut ConnectWait,
    defaults: &mut PostConnect,
    registered: &mut bool,
    plan: &ConnectionPlan,
    sts_store: Box<dyn StsStore + Send>,
    pending_events: VecDeque<Event>,
) -> ConnectionEnd {
    #[cfg(not(browser))]
    let security = if plan.address.is_secure() {
        ConnectionSecurity::Secure
    } else {
        ConnectionSecurity::Plaintext
    };
    // WebSocket security is selected by the URL and cannot be upgraded by IRC STS.
    #[cfg(browser)]
    let security = ConnectionSecurity::Secure;
    run_connection(
        stream,
        session,
        commands,
        events,
        registration,
        defaults,
        registered,
        Some(Connection {
            host: plan.address.host.clone(),
            security,
        }),
        Some(sts_store),
        plan.address.port,
        pending_events,
        &events.connected,
        true,
    )
    .await
}

fn triage_connection_end(
    mut end: ConnectionEnd,
    plan: &ConnectionPlan,
    awaiting_registration: bool,
    reconnect: Reconnect,
    attempts: &mut usize,
    last_sts_upgrade: &mut Option<String>,
) -> ControlFlow<(ConnectionEnd, bool), (ConnectionEnd, Option<u16>)> {
    if end.quit.is_some() {
        return ControlFlow::Break((end, false));
    }
    if end.reconnect_now {
        return ControlFlow::Continue((end, None));
    }
    if end.registered_at.is_some() {
        *last_sts_upgrade = None;
    }
    if let Some(port) = end.sts.upgrade {
        if let Err(error) = record_sts_upgrade(last_sts_upgrade, &plan.address.host) {
            end.reason = error.to_string();
            end.error = error;
        } else if reconnect.permits(*attempts) {
            return ControlFlow::Continue((end, Some(port)));
        }
        return ControlFlow::Break((end, true));
    }
    if awaiting_registration {
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
    events: &EventSink,
    pending_events: &mut VecDeque<Event>,
    fail_fast: bool,
    sts_required: bool,
    reconnect: Reconnect,
    attempts: usize,
    entropy: &mut (dyn FnMut() -> u64 + Send),
    last_disconnect: &mut Option<crate::DisconnectCause>,
) -> ConnectionAttempt {
    let Some(result) = connect_while_disconnected(
        connection,
        commands,
        &events.regular,
        pending_events,
        last_disconnect.as_ref(),
    )
    .await
    else {
        return ConnectionAttempt::Stop;
    };
    let error = match result {
        Ok(stream) => return ConnectionAttempt::Connected(stream),
        Err(error) => error,
    };
    if fail_fast || sts_required || is_fatal_failure(&error, None) || !reconnect.permits(attempts) {
        return ConnectionAttempt::Failed(error);
    }
    let retry_in = retry_delay(reconnect, attempts, entropy);
    let disconnected = super::disconnected_event(&error, Some(retry_in));
    *last_disconnect = Some(disconnected.cause.clone());
    if !enqueue_event(
        events,
        pending_events,
        Event::Disconnected(disconnected.clone()),
    ) {
        return ConnectionAttempt::SlowConsumer;
    }
    if wait_before_retry(commands, &events.regular, pending_events, disconnected).await {
        ConnectionAttempt::Retry
    } else {
        ConnectionAttempt::Stop
    }
}

pub(super) fn record_sts_upgrade(last_host: &mut Option<String>, host: &str) -> Result<(), Error> {
    if last_host.as_deref() == Some(host) {
        return Err(Error::Connect(crate::ConnectError::login(
            crate::ConnectErrorKind::StsUpgradeLoop,
            format!("repeated STS upgrade for {host} without completing login"),
        )));
    }
    *last_host = Some(host.to_owned());
    Ok(())
}

fn finish_connection_failure(
    events: EventSink,
    registration: &mut ConnectWait,
    error: Error,
) -> Result<(), Error> {
    let reason = error.to_string();
    if registration.0.is_some() {
        let disconnected = super::disconnected_event(&error, None);
        registration.failed(error);
        events.finish(VecDeque::from([Event::Disconnected(disconnected)]));
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
            reconnect_now: false,
        },
        false,
    )
}

fn finish_slow_consumer(
    events: EventSink,
    registration: &mut ConnectWait,
    pending_events: VecDeque<Event>,
) -> Result<(), Error> {
    registration.failed(Error::disconnected_slow_consumer());
    finish(
        events,
        ConnectionEnd {
            reason: SLOW_CONSUMER_REASON.to_owned(),
            error: Error::disconnected_slow_consumer(),
            close_kind: None,
            quit: None,
            quit_timed_out: false,
            registered_at: None,
            sts: StsActions::default(),
            sts_store: None,
            pending_events,
            reconnect_now: false,
        },
        false,
    )
}

#[cfg(not(browser))]
pub(super) fn apply_sts_action(store: &mut dyn StsStore, port: u16, action: &StsAction) {
    match action {
        StsAction::Persist { host, duration } => {
            let now = crate::rt::now_wall();
            if let Some(expires_at) = now.checked_add(*duration) {
                store.put(StsPolicy::new(host, port, expires_at, *duration));
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

fn plan_with_stored_sts(base: &ConnectionPlan, store: &mut dyn StsStore) -> ConnectionPlan {
    let Some(policy) = store.get(&base.address.host) else {
        return base.clone();
    };
    if !policy.is_active_at(crate::rt::now_wall()) {
        store.remove(&base.address.host);
        return base.clone();
    }
    if base.address.is_secure() {
        return base.clone();
    }
    let mut plan = base.clone();
    plan.address.set_tls(true);
    plan.address.set_port(policy.port);
    plan
}

#[cfg_attr(not(any(browser, tls)), allow(unused_variables))]
pub(super) fn is_fatal_failure(error: &Error, close_kind: Option<papo_core::CloseKind>) -> bool {
    #[cfg(browser)]
    if matches!(error, Error::Connect(error) if error.kind() == crate::ConnectErrorKind::Unavailable)
    {
        return true;
    }
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
        Error::Connect(error) if error.kind() == crate::ConnectErrorKind::CertificateRejected
    ) {
        return true;
    }
    false
}

async fn connect_while_disconnected(
    mut connection: ConnectFuture,
    commands: &mut mpsc::Receiver<Command>,
    events: &mpsc::Sender<Event>,
    pending_events: &mut VecDeque<Event>,
    last_cause: Option<&crate::DisconnectCause>,
) -> Option<Result<BoxConnection, Error>> {
    let disconnected = crate::Disconnected::new(
        last_cause
            .cloned()
            .unwrap_or(crate::DisconnectCause::ConnectionLost),
        None,
    );
    loop {
        tokio::select! {
            result = &mut connection => return Some(result),
            permit = events.reserve(), if !pending_events.is_empty() => match permit {
                Ok(permit) => permit.send(pending_events.pop_front().expect("pending event is present")),
                Err(_) => pending_events.clear(),
            },
            command = commands.recv() => {
                if !handle_disconnected_command(command, &disconnected) {
                    return None;
                }
            }
        }
    }
}

async fn wait_before_retry(
    commands: &mut mpsc::Receiver<Command>,
    events: &mpsc::Sender<Event>,
    pending_events: &mut VecDeque<Event>,
    disconnected: crate::Disconnected,
) -> bool {
    let deadline = now() + disconnected.retry_in.unwrap_or_default();
    let sleep = crate::rt::sleep_until(deadline);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            () = &mut sleep => return true,
            permit = events.reserve(), if !pending_events.is_empty() => match permit {
                Ok(permit) => permit.send(pending_events.pop_front().expect("pending event is present")),
                Err(_) => pending_events.clear(),
            },
            command = commands.recv() => {
                match command {
                    Some(Command::Reconnect { result }) => {
                        _ = result.send(Ok(()));
                        return true;
                    }
                    command => {
                        let current = crate::Disconnected::new(
                            disconnected.cause.clone(),
                            Some(deadline.saturating_duration_since(now())),
                        );
                        if !handle_disconnected_command(command, &current) {
                            return false;
                        }
                    }
                }
            }
        }
    }
}

fn retry_delay(
    reconnect: Reconnect,
    attempts: usize,
    entropy: &mut (dyn FnMut() -> u64 + Send),
) -> Duration {
    let retry = u32::try_from(attempts.saturating_sub(1)).unwrap_or(u32::MAX);
    reconnect.delay(retry, entropy())
}

fn handle_disconnected_command(
    command: Option<Command>,
    disconnected: &crate::Disconnected,
) -> bool {
    match command {
        Some(
            Command::Request { result, .. }
            | Command::Join { result, .. }
            | Command::CoreRequest { result, .. }
            | Command::History { result, .. }
            | Command::MarkRead { result, .. }
            | Command::ReadMarker { result, .. },
        ) => {
            _ = result.send(Err(Error::disconnected(disconnected.clone())));
            true
        }
        Some(Command::SendText { result, .. }) => {
            _ = result.send(Err(Error::disconnected(disconnected.clone())));
            true
        }
        Some(
            Command::Send { result, .. }
            | Command::Typing { result, .. }
            | Command::WaitChannelSync { result, .. }
            | Command::MonitorAdd { result, .. }
            | Command::MonitorRemove { result, .. }
            | Command::MonitorClear { result },
        ) => {
            _ = result.send(Err(Error::disconnected(disconnected.clone())));
            true
        }
        Some(Command::Reconnect { result }) => {
            _ = result.send(Ok(()));
            true
        }
        Some(Command::Quit { result, .. }) => {
            _ = result.send(Ok(()));
            false
        }
        None => false,
    }
}
