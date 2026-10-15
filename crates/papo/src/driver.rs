use std::{future::Future, io, pin::Pin, task, time::Instant};

use papo_core::Session;
use papo_proto::Message;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

use crate::{Error, Event, event, transport};

const DEFAULT_CHANNEL_CAPACITY: usize = 1024;
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

/// A cheap, cloneable handle to a running IRC client.
#[derive(Clone)]
pub struct Client {
    commands: mpsc::Sender<Command>,
}

impl Client {
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
    /// when that limit is reached.
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
/// Delivery is bounded. If events are not consumed, the driver waits and
/// applies backpressure to socket reads.
pub struct Events {
    receiver: mpsc::Receiver<Event>,
}

impl Events {
    /// Waits for the next event, or returns `None` after the driver stops.
    pub async fn next(&mut self) -> Option<Event> {
        self.receiver.recv().await
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
        let (command_sender, command_receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
        let (event_sender, event_receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
        let inner = Box::pin(run(stream, session, command_receiver, event_sender));
        (
            Client {
                commands: command_sender,
            },
            Events {
                receiver: event_receiver,
            },
            Self { inner },
        )
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
    events: mpsc::Sender<Event>,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    session.handle_connected(now());
    let mut commands_open = true;
    let mut quit = None;
    let mut buffer = [0; 4096];
    let terminal = loop {
        match drain(&mut stream, &mut session, &events).await {
            Ok(Some(reason)) => break reason,
            Ok(None) => {}
            Err(error) => {
                return finish(
                    stream,
                    session,
                    events,
                    error.to_string(),
                    Error::Io(error),
                    quit,
                    false,
                )
                .await;
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
            read = stream.read(&mut buffer) => match read {
                Ok(0) => break "server closed the connection".to_owned(),
                Ok(count) => session.handle_bytes(now(), &buffer[..count]),
                Err(error) => return finish(stream, session, events, error.to_string(), Error::Io(error), quit, false).await,
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
    events: &mpsc::Sender<Event>,
) -> io::Result<Option<String>>
where
    S: AsyncWrite + Unpin,
{
    while let Some(line) = session.poll_transmit() {
        stream.write_all(&line).await?;
    }
    while let Some(core_event) = session.poll_event() {
        match event::from_core(core_event) {
            event::CoreEvent::Forward(event) => _ = events.send(event).await,
            event::CoreEvent::Close(reason) => return Ok(Some(reason)),
            event::CoreEvent::Unknown => {}
        }
    }
    Ok(None)
}

async fn finish<S>(
    mut stream: S,
    mut session: Session,
    events: mpsc::Sender<Event>,
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
    _ = events.send(Event::Disconnected { reason }).await;
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

fn now() -> Instant {
    tokio::time::Instant::now().into_std()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use papo_core::Config;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

    use super::{Driver, Error, Event, Session};

    fn scripted(config: Config) -> (DuplexStream, super::Client, super::Events, Driver) {
        let (client_stream, server) = tokio::io::duplex(4096);
        let (client, events, driver) = Driver::with_stream(client_stream, Session::new(config));
        (server, client, events, driver)
    }

    async fn registration_burst(server: &mut DuplexStream) {
        let expected = b"CAP LS 302\r\nNICK papo-driver\r\nUSER papo 0 * :papo driver test\r\n";
        let mut actual = vec![0; expected.len()];
        server.read_exact(&mut actual).await.unwrap();
        assert_eq!(actual, expected);
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
