use std::{future::Future, io, pin::Pin, task, time::Instant};

use papo_core::Session;
use papo_proto::Message;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

use crate::{
    AddressError, Error, Event,
    address::{Protocol, ServerAddress},
    event,
    transport::{self, TlsConfig},
};

pub(crate) const DEFAULT_CHANNEL_CAPACITY: usize = 1024;
const QUIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

enum Command {
    Send {
        message: Message,
        result: oneshot::Sender<Result<(), papo_proto::BuildError>>,
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
}

struct Channels {
    client: Client,
    events: Events,
    commands: mpsc::Receiver<Command>,
    event_sink: EventSink,
}

pub(crate) struct ConnectionPlan {
    pub(crate) address: ServerAddress,
    pub(crate) tls: TlsConfig,
}

/// A cheap, cloneable handle to a running IRC client.
#[derive(Clone)]
pub struct Client {
    commands: mpsc::Sender<Command>,
}

impl Client {
    /// Starts configuring a client for `address`.
    pub fn builder(address: &str) -> crate::Builder {
        crate::Builder::new(address)
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
        receiver.await.map_err(|_| Error::Disconnected)??;
        Ok(())
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
        let channels = channels(capacity);
        let inner = Box::pin(run(stream, session, channels.commands, channels.event_sink));
        (channels.client, channels.events, Self { inner })
    }

    pub(crate) fn with_plan(
        plan: Result<ConnectionPlan, AddressError>,
        session: Session,
        capacity: usize,
    ) -> (Client, Events, Self) {
        let channels = channels(capacity);
        let inner = Box::pin(async move {
            let stream = open_connection(plan.map_err(Error::Address)?).await?;
            run(stream, session, channels.commands, channels.event_sink).await
        });
        (channels.client, channels.events, Self { inner })
    }
}

fn channels(capacity: usize) -> Channels {
    let (command_sender, command_receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
    let (event_sender, event_receiver) = mpsc::channel(capacity);
    let (disconnect_sender, disconnect_receiver) = oneshot::channel();
    let events = EventSink {
        regular: event_sender,
        disconnected: disconnect_sender,
    };
    Channels {
        client: Client {
            commands: command_sender,
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

async fn run<S>(
    mut stream: S,
    mut session: Session,
    mut commands: mpsc::Receiver<Command>,
    events: EventSink,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    session.handle_connected(now());
    let mut commands_open = true;
    let mut quit = None;
    let mut pending_event = None;
    let mut buffer = [0; 4096];
    let terminal = loop {
        match drain(&mut stream, &mut session, &mut pending_event).await {
            Ok(Some(reason)) => break reason,
            Ok(None) => {}
            Err(error) => {
                return finish_io(stream, session, events, error, quit).await;
            }
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
                Ok(0) => break "server closed the connection".to_owned(),
                Ok(count) => session.handle_bytes(now(), &buffer[..count]),
                Err(error) => return finish_io(stream, session, events, error, quit).await,
            },
            permit = events.regular.clone().reserve_owned(), if pending_event.is_some() => match permit {
                Ok(permit) => {
                    _ = permit.send(pending_event.take().expect("pending event is present"));
                }
                Err(_) => pending_event = None,
            },
            command = commands.recv(), if commands_open => match command {
                Some(Command::Send { message, result }) => {
                    _ = result.send(session.send(now(), message));
                }
                Some(Command::Quit { reason, result }) => {
                    request_quit(&mut session, &mut quit, reason, Some(result));
                }
                None => {
                    commands_open = false;
                    request_quit(&mut session, &mut quit, None, None);
                }
            },
            () = wait_until(deadline) => {
                let current = now();
                if quit.as_ref().is_some_and(|quit| current >= quit.deadline) {
                    return finish(
                        stream,
                        session,
                        events,
                        "QUIT was not acknowledged within 10 seconds".to_owned(),
                        Error::Timeout,
                        quit,
                        true,
                    ).await;
                }
                session.handle_timeout(current);
            },
        }
    };
    finish(
        stream,
        session,
        events,
        terminal,
        Error::Disconnected,
        quit,
        false,
    )
    .await
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

async fn drain<S>(
    stream: &mut S,
    session: &mut Session,
    pending_event: &mut Option<Event>,
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
            event::CoreEvent::Forward(event) => *pending_event = Some(event),
            event::CoreEvent::Close(reason) => return Ok(Some(reason)),
            event::CoreEvent::Unknown => {}
        }
    }
    Ok(None)
}

async fn finish<S>(
    mut stream: S,
    mut session: Session,
    events: EventSink,
    mut reason: String,
    mut result: Error,
    quit: Option<Quit>,
    quit_timed_out: bool,
) -> Result<(), Error>
where
    S: AsyncWrite + Unpin,
{
    session.handle_disconnected(now());
    let close_error = stream.flush().await.err().or(stream.shutdown().await.err());
    if let Some(error) = close_error
        && !matches!(result, Error::Io(_))
    {
        reason = error.to_string();
        result = Error::Io(error);
    }
    drop(events.regular);
    _ = events.disconnected.send(Event::Disconnected { reason });
    if let Some(quit) = quit {
        for response in quit.results {
            _ = response.send(if quit_timed_out {
                Err(Error::Timeout)
            } else {
                Ok(())
            });
        }
        Ok(())
    } else {
        Err(result)
    }
}

async fn finish_io<S>(
    stream: S,
    session: Session,
    events: EventSink,
    error: io::Error,
    quit: Option<Quit>,
) -> Result<(), Error>
where
    S: AsyncWrite + Unpin,
{
    let reason = error.to_string();
    finish(
        stream,
        session,
        events,
        reason,
        Error::Io(error),
        quit,
        false,
    )
    .await
}

fn now() -> Instant {
    tokio::time::Instant::now().into_std()
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        task::{Context, Poll},
        time::Duration,
    };

    use papo_core::{
        Config, Decoding, Sasl,
        sts::{ConnectionSecurity, StsAction},
    };
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};

    use super::{Driver, Error, Event, Session};

    struct FailingStream {
        inner: DuplexStream,
        read_error: Arc<AtomicBool>,
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

    fn config() -> Config {
        Config::default()
            .nick("papo-driver")
            .realname("papo driver test")
            .wanted_caps([] as [&str; 0])
    }

    #[tokio::test(flavor = "current_thread")]
    async fn registration_is_forwarded() {
        let (mut server, _client, mut events, driver) = scripted(config());
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
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
                reason: "server closed the connection".into()
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
                reason: "shutdown failed".into()
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
                reason: "original read failure".into()
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
                reason: "incoming IRC line exceeds 8703 bytes".into()
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
                reason: "sts upgrade".into()
            })
        );
        assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
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
                reason: "original read failure".into()
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
                reason: "QUIT was not acknowledged within 10 seconds".into()
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
