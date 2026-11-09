use std::{
    collections::VecDeque,
    ops::ControlFlow,
    sync::{Arc, RwLock},
    time::Duration,
};

use papo_core::sts::StsAction;
use papo_core::{
    Connection, Session,
    sts::{ConnectionSecurity, StsPolicy, StsStore, reschedule_on_disconnect},
};
use tokio::sync::mpsc;

use super::{
    BackoffEntropy, BoxConnection, Command, ConnectFuture, ConnectWait, ConnectionEnd,
    ConnectionPlan, Connector, EventSink, PendingEvents, PostConnect, ReconnectControl, StsActions,
    enqueue_event, finish, now, run_connection,
};
#[cfg(sockets)]
use crate::address::Protocol;
use crate::{Error, Event, InvalidInput, Reconnect};

#[cfg(any(sockets, browser))]
#[allow(unused_variables)]
pub(super) async fn open_connection(
    plan: ConnectionPlan,
    progress: mpsc::UnboundedSender<papo_core::ConnectPhase>,
) -> Result<crate::transport::stream::Stream, Error> {
    use crate::transport;

    #[cfg(sockets)]
    {
        if let Some(url) = plan.address.websocket_url() {
            #[cfg(feature = "websocket")]
            {
                if plan.address.is_secure() {
                    _ = progress.send(papo_core::ConnectPhase::Tls);
                }
                return transport::connect_websocket(
                    &url,
                    plan.address.is_secure().then_some(&plan.tls),
                )
                .await
                .map_err(Error::transport);
            }
            #[cfg(not(feature = "websocket"))]
            {
                _ = url;
                unreachable!("connection plans reject WebSocket endpoints without support");
            }
        }
        let security = match plan.address.protocol {
            Protocol::Irc => transport::Security::Plaintext,
            #[cfg(tls)]
            Protocol::Ircs => {
                _ = progress.send(papo_core::ConnectPhase::Tls);
                transport::Security::Tls(plan.tls)
            }
            #[cfg(not(tls))]
            Protocol::Ircs => unreachable!("connection plans reject TLS without a provider"),
            Protocol::Ws | Protocol::Wss => {
                unreachable!("WebSocket endpoints return before TCP transport selection")
            }
        };
        transport::connect(&plan.address.host, plan.address.port, &security)
            .await
            .map_err(Error::transport)
    }
    #[cfg(browser)]
    {
        let url = plan
            .address
            .websocket_url()
            .expect("browser connection plans use WebSocket endpoints");
        if plan.address.is_secure() {
            _ = progress.send(papo_core::ConnectPhase::Tls);
        }
        transport::browser::connect_websocket(&url, None)
            .await
            .map_err(Error::transport)
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
    connect_timeout: Duration,
    mut sts_store: Arc<dyn StsStore>,
    connector: Connector,
    mut entropy: BackoffEntropy,
    network_builder: Option<Arc<RwLock<crate::builder::NetworkBuilderSettings>>>,
) -> Result<(), Error> {
    let Some(mut base_plan) = initial_connection_plan(plan, &mut registration)? else {
        return Ok(());
    };
    let mut attempts = 0_usize;
    let mut registered = false;
    let mut reconnect_control = ReconnectControl::default();
    let mut pending_events = VecDeque::new();
    let mut sts_required = false;
    let mut last_sts_upgrade = None;
    let mut last_disconnect = None;
    loop {
        attempts = attempts.saturating_add(1);
        let plan = plan_with_stored_sts(&base_plan, sts_store.as_ref());
        if let Some(settings) = &network_builder {
            settings
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .address = Ok(plan.address.clone());
        }
        sts_required |= !base_plan.address.is_secure() && plan.address.is_secure();
        let phase = crate::ConnectPhase::from_core(papo_core::ConnectPhase::Transport)
            .expect("transport is a public connection phase");
        enqueue_event(
            &events,
            &mut pending_events,
            Event::Connecting(crate::event::Connecting {
                attempt: attempts,
                phase,
            }),
        );
        let (progress, progress_events) = mpsc::unbounded_channel();
        let stream = match connect_or_wait(
            connector(plan.clone(), progress),
            progress_events,
            &mut commands,
            &events,
            &mut pending_events,
            &mut defaults,
            &mut reconnect_control,
            registration.0.is_some(),
            sts_required,
            connect_timeout,
            reconnect,
            attempts,
            registered,
            entropy.as_mut(),
            &mut last_disconnect,
        )
        .await
        {
            ConnectionAttempt::Connected(stream) => stream,
            ConnectionAttempt::Retry => continue,
            ConnectionAttempt::Stop => {
                finish_requested(events, pending_events, attempts.saturating_add(1));
                return Ok(());
            }
            ConnectionAttempt::Failed(error) => {
                return finish_connection_failure(
                    events,
                    &mut registration,
                    error,
                    attempts.saturating_add(1),
                );
            }
        };
        let mut end = run_planned_connection(
            stream,
            &mut session,
            &mut commands,
            &events,
            &mut registration,
            &mut defaults,
            &mut reconnect_control,
            &mut registered,
            attempts,
            &plan,
            sts_store,
            std::mem::take(&mut pending_events),
        )
        .await;
        if end.registered_at.is_some() {
            defaults.disconnected_at = Some(crate::rt::now_wall());
            attempts = 0;
        }
        sts_store = end
            .sts_store
            .take()
            .expect("planned connections retain their STS store");
        reschedule_stored_policy(sts_store.as_ref(), &plan.address.host);
        let (end, upgrade_port) = match triage_connection_end(
            end,
            &plan,
            registration.0.is_some(),
            reconnect,
            &mut attempts,
            &mut last_sts_upgrade,
        ) {
            ControlFlow::Break((mut end, login_failed)) => {
                if login_failed {
                    let awaiting_registration = registration.0.is_some();
                    let error = end.login_error(awaiting_registration);
                    registration.failed(error.clone());
                    end.error = error;
                }
                return finish(events, end, attempts.saturating_add(1));
            }
            ControlFlow::Continue(retry) => retry,
        };
        if let Some(port) = upgrade_port {
            let mut upgraded = base_plan.address.clone();
            upgraded.apply_sts_upgrade(port);
            if !upgraded.is_supported() {
                let error = Error::Connect(crate::ConnectError::login(
                    crate::ConnectErrorKind::HandshakeFailed,
                    "the server requested a TLS upgrade that this build cannot open",
                ));
                return finish_connection_failure(
                    events,
                    &mut registration,
                    error,
                    attempts.saturating_add(1),
                );
            }
            base_plan.address = upgraded;
            sts_required = true;
        }
        let refused = retry_was_refused(&end);
        let reconnect_in = if upgrade_port.is_some() {
            Duration::ZERO
        } else {
            retry_delay(reconnect, attempts, registered, refused, entropy.as_mut())
        };
        let disconnected =
            super::disconnected_event(&end.error, attempts.saturating_add(1), Some(reconnect_in));
        last_disconnect = Some(disconnected.cause.clone());
        pending_events = enqueue_retry_event(&events, end, disconnected.clone());
        if upgrade_port.is_some() {
            continue;
        }
        if !wait_before_retry(
            &mut commands,
            &mut defaults,
            &mut reconnect_control,
            disconnected,
        )
        .await
        {
            finish_requested(events, pending_events, attempts.saturating_add(1));
            return Ok(());
        }
    }
}

fn finish_requested(events: EventSink, mut pending_events: PendingEvents, attempt: usize) {
    pending_events.push_back(super::event_item(
        &events,
        Event::Disconnected(crate::Disconnected::new(
            crate::DisconnectCause::Requested {
                quit_unacknowledged: false,
            },
            attempt,
            None,
        )),
    ));
    events.finish(pending_events);
}

fn enqueue_retry_event(
    events: &EventSink,
    mut end: ConnectionEnd,
    disconnected: crate::Disconnected,
) -> PendingEvents {
    enqueue_event(
        events,
        &mut end.pending_events,
        Event::Disconnected(disconnected),
    );
    end.pending_events
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
    reconnect_control: &mut ReconnectControl,
    registered: &mut bool,
    attempt: usize,
    plan: &ConnectionPlan,
    sts_store: Arc<dyn StsStore>,
    pending_events: PendingEvents,
) -> ConnectionEnd {
    // WebSocket security is selected by the URL and cannot be upgraded by IRC STS.
    let security = if cfg!(browser) || plan.address.is_secure() {
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
        reconnect_control,
        registered,
        attempt,
        Some(Connection {
            host: plan.address.host.clone(),
            security,
            authenticated_server_name: authenticated_server_name(plan),
        }),
        Some(sts_store),
        plan.address.port,
        plan.address.is_websocket(),
        pending_events,
    )
    .await
}

fn authenticated_server_name(plan: &ConnectionPlan) -> Option<String> {
    if !plan.address.is_secure() {
        return None;
    }
    let name = plan.tls.server_name_or(&plan.address.host);
    Some(crate::sts::normalize_host(name))
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
    if end.registered_at.is_none() {
        end.error = end.login_error(true);
    }
    if awaiting_registration {
        return ControlFlow::Break((end, true));
    }
    *last_sts_upgrade = None;
    if is_fatal_failure(&end.error, end.close_kind) {
        return ControlFlow::Break((end, true));
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
    progress: mpsc::UnboundedReceiver<papo_core::ConnectPhase>,
    commands: &mut mpsc::Receiver<Command>,
    events: &EventSink,
    pending_events: &mut PendingEvents,
    defaults: &mut PostConnect,
    reconnect_control: &mut ReconnectControl,
    fail_fast: bool,
    sts_required: bool,
    connect_timeout: Duration,
    reconnect: Reconnect,
    attempts: usize,
    after_connected: bool,
    entropy: &mut (dyn FnMut() -> u64 + Send),
    last_disconnect: &mut Option<crate::DisconnectCause>,
) -> ConnectionAttempt {
    let result = match connect_while_disconnected(
        connection,
        progress,
        commands,
        events,
        pending_events,
        defaults,
        reconnect_control,
        last_disconnect.as_ref(),
        connect_timeout,
        attempts,
    )
    .await
    {
        DisconnectedWait::Complete(result) => result,
        DisconnectedWait::Stop => return ConnectionAttempt::Stop,
    };
    let error = match result {
        Ok(stream) => return ConnectionAttempt::Connected(stream),
        Err(error) => error,
    };
    let sts_tls_failure = sts_required
        && matches!(
            error,
            Error::Connect(ref error) if error.is_sts_tls_failure()
        );
    if fail_fast
        || sts_tls_failure
        || is_fatal_failure(&error, None)
        || !reconnect.permits(attempts)
    {
        return ConnectionAttempt::Failed(error);
    }
    let reconnect_in = retry_delay(reconnect, attempts, after_connected, false, entropy);
    let disconnected =
        super::disconnected_event(&error, attempts.saturating_add(1), Some(reconnect_in));
    *last_disconnect = Some(disconnected.cause.clone());
    enqueue_event(
        events,
        pending_events,
        Event::Disconnected(disconnected.clone()),
    );
    if wait_before_retry(commands, defaults, reconnect_control, disconnected).await {
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
    attempt: usize,
) -> Result<(), Error> {
    let reason = error.to_string();
    if registration.0.is_some() {
        let disconnected = super::disconnected_event(&error, attempt, None);
        registration.failed(error);
        let pending = VecDeque::from([super::event_item(
            &events,
            Event::Disconnected(disconnected),
        )]);
        events.finish(pending);
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
        attempt,
    )
}

pub(super) fn apply_sts_action(
    store: &dyn StsStore,
    port: u16,
    websocket: bool,
    action: &StsAction,
) {
    match action {
        StsAction::Persist { host, duration, .. } => {
            let now = crate::rt::now_wall();
            let expires_at = clamped_sts_expiry(now, *duration);
            put_sts_policy(
                store,
                StsPolicy::new(host, port, expires_at, *duration).websocket(websocket),
            );
        }
        StsAction::Remove { host, .. } => remove_sts_policy(store, host),
        _ => {}
    }
}

fn clamped_sts_expiry(now: std::time::SystemTime, duration: Duration) -> std::time::SystemTime {
    if let Some(expires_at) = now.checked_add(duration) {
        return expires_at;
    }
    let mut seconds = duration.as_secs();
    while seconds > 0 {
        seconds /= 2;
        if let Some(expires_at) = now.checked_add(Duration::from_secs(seconds)) {
            return expires_at;
        }
    }
    now
}

pub(super) fn reschedule_stored_policy(store: &dyn StsStore, host: &str) {
    if let Some(policy) = get_sts_policy(store, host) {
        put_sts_policy(
            store,
            reschedule_on_disconnect(policy, crate::rt::now_wall()),
        );
    }
}

pub(super) fn plan_with_stored_sts(base: &ConnectionPlan, store: &dyn StsStore) -> ConnectionPlan {
    let Some(policy) = get_sts_policy(store, &base.address.host) else {
        return base.clone();
    };
    if !policy.is_active_at(crate::rt::now_wall()) {
        remove_sts_policy(store, &base.address.host);
        return base.clone();
    }
    if base.address.is_secure() {
        return base.clone();
    }
    let mut plan = base.clone();
    plan.address.apply_stored_sts(&policy);
    plan
}

fn get_sts_policy(store: &dyn StsStore, host: &str) -> Option<StsPolicy> {
    store.get(&crate::sts::normalize_host(host))
}

fn put_sts_policy(store: &dyn StsStore, mut policy: StsPolicy) {
    policy.host = crate::sts::normalize_host(&policy.host);
    store.put(policy);
}

fn remove_sts_policy(store: &dyn StsStore, host: &str) {
    store.remove(&crate::sts::normalize_host(host));
}

pub(super) fn is_fatal_failure(error: &Error, close_kind: Option<papo_core::CloseKind>) -> bool {
    if matches!(error, Error::Connect(error) if error.is_configuration_failure()) {
        return true;
    }
    if matches!(
        close_kind,
        Some(
            papo_core::CloseKind::SaslFailed
                | papo_core::CloseKind::AccountRequired
                | papo_core::CloseKind::PasswordIncorrect
                | papo_core::CloseKind::ConnectionRestricted
                | papo_core::CloseKind::Banned
                | papo_core::CloseKind::WebIrcRejected
                | papo_core::CloseKind::BouncerBindingFailed
                | papo_core::CloseKind::CapabilityUnavailable
        )
    ) {
        return true;
    }
    if matches!(error, Error::Connect(error) if error.is_certificate_rejected()) {
        return true;
    }
    false
}

enum DisconnectedWait {
    Complete(Result<BoxConnection, Error>),
    Stop,
}

enum DisconnectedAction {
    Wait,
    Retry,
    Stop,
}

#[allow(clippy::too_many_arguments)]
async fn connect_while_disconnected(
    mut connection: ConnectFuture,
    mut progress: mpsc::UnboundedReceiver<papo_core::ConnectPhase>,
    commands: &mut mpsc::Receiver<Command>,
    events: &EventSink,
    pending_events: &mut PendingEvents,
    defaults: &mut PostConnect,
    reconnect_control: &mut ReconnectControl,
    last_cause: Option<&crate::DisconnectCause>,
    timeout: Duration,
    attempt: usize,
) -> DisconnectedWait {
    let disconnected = crate::Disconnected::new(
        last_cause.cloned().unwrap_or_else(|| {
            crate::DisconnectCause::connection_lost("transport ended unexpectedly")
        }),
        attempt,
        None,
    );
    let timeout = super::wait_until(papo_core::deadline(now(), timeout));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            biased;
            Some(phase) = progress.recv() => {
                let Some(phase) = crate::ConnectPhase::from_core(phase) else {
                    continue;
                };
                enqueue_event(events, pending_events, Event::Connecting(
                    crate::event::Connecting { attempt, phase },
                ));
            }
            result = &mut connection => {
                while let Ok(phase) = progress.try_recv() {
                    let Some(phase) = crate::ConnectPhase::from_core(phase) else {
                        continue;
                    };
                    enqueue_event(events, pending_events, Event::Connecting(
                        crate::event::Connecting { attempt, phase },
                    ));
                }
                return DisconnectedWait::Complete(result);
            }
            () = &mut timeout => {
                return DisconnectedWait::Complete(Err(Error::Connect(crate::ConnectError::login(
                    crate::ConnectErrorKind::TimedOut,
                    "connection attempt timed out",
                ))));
            }
            command = commands.recv() => {
                if matches!(
                    handle_disconnected_command(
                        command,
                        &disconnected,
                        defaults,
                        reconnect_control,
                    ),
                    DisconnectedAction::Stop
                ) {
                    return DisconnectedWait::Stop;
                }
            }
        }
    }
}

async fn wait_before_retry(
    commands: &mut mpsc::Receiver<Command>,
    defaults: &mut PostConnect,
    reconnect_control: &mut ReconnectControl,
    disconnected: crate::Disconnected,
) -> bool {
    let deadline = papo_core::deadline(now(), disconnected.reconnect_in.unwrap_or_default());
    let sleep = super::wait_until(deadline);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            biased;
            command = commands.recv() => {
                let current = crate::Disconnected::new(
                    disconnected.cause.clone(),
                    disconnected.attempt,
                    Some(deadline.map_or(Duration::MAX, |deadline| {
                        deadline.saturating_duration_since(now())
                    })),
                );
                match handle_disconnected_command(
                    command,
                    &current,
                    defaults,
                    reconnect_control,
                ) {
                    DisconnectedAction::Wait => {}
                    DisconnectedAction::Retry => return true,
                    DisconnectedAction::Stop => return false,
                }
            }
            () = &mut sleep, if !reconnect_control.paused => return true,
        }
    }
}

fn retry_delay(
    reconnect: Reconnect,
    attempts: usize,
    after_connected: bool,
    refused: bool,
    entropy: &mut (dyn FnMut() -> u64 + Send),
) -> Duration {
    let retry =
        u32::try_from(attempts.saturating_sub(usize::from(!after_connected))).unwrap_or(u32::MAX);
    if refused {
        reconnect.refused(retry, entropy())
    } else {
        reconnect.delay(retry, entropy())
    }
}

fn retry_was_refused(end: &ConnectionEnd) -> bool {
    end.registered_at.is_none()
        && matches!(
            end.error,
            Error::Connect(ref error)
                if matches!(
                    error.kind(),
                    crate::ConnectErrorKind::Server | crate::ConnectErrorKind::SaslUnavailable
                )
        )
}

fn handle_disconnected_command(
    command: Option<Command>,
    disconnected: &crate::Disconnected,
    defaults: &mut PostConnect,
    reconnect_control: &mut ReconnectControl,
) -> DisconnectedAction {
    match command {
        Some(Command::Composite { task }) => {
            _ = crate::rt::spawn(task);
            DisconnectedAction::Wait
        }
        Some(
            Command::Request { result, .. }
            | Command::Join { result, .. }
            | Command::CoreRequest { result, .. }
            | Command::History { result, .. }
            | Command::MarkRead { result, .. }
            | Command::ReadMarker { result, .. },
        ) => {
            _ = result.send(Err(Error::disconnected(disconnected.clone())));
            DisconnectedAction::Wait
        }
        Some(Command::Part {
            channel, result, ..
        }) => {
            defaults
                .channels
                .retain(|entry| channel != entry.channel.as_str());
            if !defaults
                .parted_while_disconnected
                .iter()
                .any(|parted| parted == channel.as_str())
            {
                defaults.parted_while_disconnected.push(channel);
            }
            _ = result.send(Err(Error::disconnected(disconnected.clone())));
            DisconnectedAction::Wait
        }
        Some(Command::SendText { result, .. }) => {
            _ = result.send(Err(Error::disconnected(disconnected.clone())));
            DisconnectedAction::Wait
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
            DisconnectedAction::Wait
        }
        Some(Command::Reconnect { result }) => {
            _ = result.send(Ok(()));
            DisconnectedAction::Retry
        }
        Some(Command::PauseReconnect { result }) => {
            reconnect_control.paused = true;
            _ = result.send(Ok(()));
            DisconnectedAction::Wait
        }
        Some(Command::ResumeReconnect { result }) => {
            let was_paused = std::mem::replace(&mut reconnect_control.paused, false);
            _ = result.send(Ok(()));
            if was_paused {
                DisconnectedAction::Retry
            } else {
                DisconnectedAction::Wait
            }
        }
        Some(Command::DirectAcquire { result, .. }) => {
            _ = result.send(Err(Error::disconnected(disconnected.clone())));
            DisconnectedAction::Wait
        }
        Some(Command::DirectProve { result, .. }) => {
            _ = result.send(Err(Error::disconnected(disconnected.clone())));
            DisconnectedAction::Wait
        }
        Some(Command::DirectRelease { .. }) => DisconnectedAction::Wait,
        Some(Command::Quit { result, .. }) => {
            _ = result.send(Ok(()));
            DisconnectedAction::Stop
        }
        None => DisconnectedAction::Stop,
    }
}

#[cfg(all(test, sockets))]
mod tests {
    use std::io;

    use super::*;
    use crate::transport::TlsConfig;

    fn connection_end(error: Error, registered: bool) -> ConnectionEnd {
        ConnectionEnd {
            reason: error.to_string(),
            error,
            close_kind: None,
            quit: None,
            quit_timed_out: false,
            registered_at: registered.then(now),
            sts: StsActions::default(),
            sts_store: None,
            pending_events: VecDeque::new(),
        }
    }

    #[test]
    fn refused_and_transport_retries_use_distinct_delays() {
        let refusal = connection_end(
            Error::Connect(crate::ConnectError::login(
                crate::ConnectErrorKind::Server,
                "Too many clients from your network",
            )),
            false,
        );
        let transport_after_welcome = connection_end(
            Error::connection_lost_with("connection reset by peer"),
            true,
        );
        let transport_before_welcome = connection_end(
            Error::Connect(crate::ConnectError::new(
                crate::ConnectErrorKind::Refused,
                io::Error::new(io::ErrorKind::ConnectionRefused, "connection refused"),
            )),
            false,
        );
        let reconnect = Reconnect::exponential()
            .initial_delay(Duration::from_secs(5))
            .refused_delay(Duration::from_secs(47));

        assert!(retry_was_refused(&refusal));
        assert!(!retry_was_refused(&transport_after_welcome));
        assert!(!retry_was_refused(&transport_before_welcome));
        assert_eq!(
            retry_delay(reconnect, 1, false, retry_was_refused(&refusal), &mut || 0,),
            Duration::from_secs(47)
        );
        assert_eq!(
            retry_delay(
                reconnect,
                1,
                false,
                retry_was_refused(&transport_after_welcome),
                &mut || 0,
            ),
            Duration::from_secs(4)
        );
    }

    #[test]
    fn only_verified_transport_plans_authenticate_a_server_name() {
        fn plan(address: &str) -> ConnectionPlan {
            ConnectionPlan {
                address: address.parse().unwrap(),
                tls: TlsConfig::default(),
            }
        }

        assert_eq!(authenticated_server_name(&plan("irc://irc.example")), None);
        assert_eq!(authenticated_server_name(&plan("ws://irc.example")), None);
        assert_eq!(
            authenticated_server_name(&plan("ircs://IRC.Example")),
            Some("irc.example".to_owned())
        );
        assert_eq!(
            authenticated_server_name(&plan("wss://IRC.Example:8443/webirc")),
            Some("irc.example".to_owned())
        );
        assert_eq!(
            authenticated_server_name(&plan("wss://irc.example:9443/other")),
            Some("irc.example".to_owned())
        );

        let mut overridden = plan("ircs://edge.example");
        overridden.tls.server_name = Some("Identity.Example.".to_owned());
        assert_eq!(
            authenticated_server_name(&overridden),
            Some("identity.example".to_owned())
        );
    }
}

#[cfg(all(test, browser))]
mod browser_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::{ConnectionPlan, authenticated_server_name};
    use crate::transport::TlsConfig;

    #[test]
    #[wasm_bindgen_test]
    fn secure_websocket_authenticates_its_normalized_hostname() {
        let plan = ConnectionPlan {
            address: "wss://IRC.Example:8443/webirc".parse().unwrap(),
            tls: TlsConfig::default(),
        };
        assert_eq!(
            authenticated_server_name(&plan),
            Some("irc.example".to_owned())
        );
    }
}
