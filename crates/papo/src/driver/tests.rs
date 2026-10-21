use super::test_support::*;
use std::fmt::Write as _;

macro_rules! disconnected {
    () => {
        Error::Disconnected(_)
    };
}

fn transport_failure(message: &'static str) -> Error {
    Error::Connect(crate::ConnectError::new(
        crate::ConnectErrorKind::Refused,
        io::Error::new(io::ErrorKind::ConnectionRefused, message),
    ))
}

fn test_nick(value: &str) -> crate::Nick {
    crate::Casemapping::default().nick(value)
}

fn test_channel(value: &str) -> crate::ChannelName {
    crate::Casemapping::default().channel(value)
}

fn test_mask(value: &str) -> crate::Mask {
    crate::Casemapping::default().mask(value)
}

fn test_account(value: &str) -> crate::AccountName {
    crate::Casemapping::default().account(value)
}

async fn assert_action_refusal<T: std::fmt::Debug + Send + 'static>(
    server: &mut DuplexStream,
    action: crate::RequestFuture<T>,
    outgoing: &str,
    incoming: &str,
    numeric: u16,
    refusal: crate::Refusal,
) {
    let action = tokio::spawn(action);
    client_line(server, outgoing.as_bytes()).await;
    server_line(server, incoming.as_bytes()).await;
    let error = within(action).await.unwrap().unwrap_err();
    assert!(matches!(
        error,
        Error::Server(error)
            if error.numeric() == Some(numeric) && error.refusal() == refusal
    ));
}

struct NeverWrite;

impl AsyncWrite for NeverWrite {
    fn poll_write(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        _buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Pending
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

struct StallingWriter {
    written: Vec<u8>,
    first_write: usize,
    stalled: bool,
}

struct NeverDrainingStream {
    inner: DuplexStream,
    written: Arc<Mutex<Vec<u8>>>,
}

impl AsyncRead for NeverDrainingStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(context, buffer)
    }
}

impl AsyncWrite for NeverDrainingStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.inner).poll_write(context, buffer) {
            Poll::Ready(Ok(count)) => {
                self.written
                    .lock()
                    .unwrap()
                    .extend_from_slice(&buffer[..count]);
                Poll::Ready(Ok(count))
            }
            result => result,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }

    fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

impl AsyncWrite for StallingWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.stalled {
            return Poll::Pending;
        }
        let count = if self.first_write == 0 {
            buffer.len()
        } else {
            self.first_write.min(buffer.len())
        };
        self.written.extend_from_slice(&buffer[..count]);
        if self.first_write != 0 {
            self.first_write = 0;
            self.stalled = true;
        }
        Poll::Ready(Ok(count))
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test(flavor = "current_thread")]
async fn write_deadline_interrupts_a_socket_that_never_accepts_data() {
    let deadline = crate::rt::now_mono() + Duration::from_millis(1);
    let mut pending = super::PendingWrite {
        buffer: b"QUIT\r\n".to_vec(),
        offset: 0,
        token: None,
    };
    assert!(
        !super::write_before(&mut NeverWrite, &mut pending, Some(deadline))
            .await
            .unwrap()
    );
    assert_eq!(pending.offset, 0);
}

#[test]
fn shared_test_deadline_is_five_seconds() {
    assert_eq!(TEST_DEADLINE, Duration::from_secs(5));
}

#[cfg(not(target_family = "wasm"))]
#[tokio::test(flavor = "current_thread")]
async fn shared_test_deadline_rejects_a_stalled_future() {
    let waiting = tokio::spawn(within_duration(
        std::future::pending::<()>(),
        Duration::from_millis(1),
    ));
    let result = tokio::time::timeout(Duration::from_secs(1), waiting)
        .await
        .expect("deadline helper did not finish");
    assert!(result.unwrap_err().is_panic());
}

#[tokio::test(flavor = "current_thread")]
async fn message_fields_use_captured_session_facts() {
    let (mut server, _client, mut events, driver) = scripted(config().max_batch_lines(0));
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
    client_line(&mut server, b"CAP END\r\n").await;
    server_line(&mut server, b":ergo.test 005 papo-driver CHANTYPES=# CASEMAPPING=rfc1459 :are supported\r\n:ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :No MOTD\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Connected(_))));

    server_line(&mut server, b"@time=2026-02-03T04:05:06.789Z;msgid=RfcBTzyxY120004;account=emily;bot;+draft/reply=RfcBTzyxY120003 :emily!~e@localhost PRIVMSG #papo :hello \x02from Ergo\x02\r\n").await;
    let message = next_message(&mut events).await;
    assert_eq!(message.kind, MessageKind::Privmsg);
    assert_eq!(message.text, "hello from Ergo");
    assert_eq!(message.raw_text, "hello \x02from Ergo\x02");
    assert!(
        matches!(&message.conversation, crate::Conversation::Channel(channel) if channel == "#papo")
    );
    assert!(!matches!(
        message.conversation,
        crate::Conversation::Private(_)
    ));
    assert_eq!(message.return_address(), "#papo");
    assert_eq!(message.msgid.as_deref(), Some("RfcBTzyxY120004"));
    assert_eq!(message.account.as_deref(), Some("emily"));
    assert!(message.is_bot);
    assert_eq!(
        message.reply_to().map(AsRef::as_ref),
        Some("RfcBTzyxY120003")
    );
    assert_eq!(
        message.time(),
        UNIX_EPOCH + Duration::new(1_770_091_506, 789_000_000)
    );
    assert_eq!(message.origin, Origin::Live);
    assert!(message.is_live());

    server_line(&mut server, b":emily!~e@localhost NOTICE papo-driver :maintenance soon\r\n:emily!~e@localhost PRIVMSG #papo :\x01ACTION deploys \x02bold\x02 and \x1ditalic\x1d then \x02\x1dboth\x0f in \x0304red and \x04FF0000hex\x01\r\n:dan!~d@cloak PRIVMSG #papo :\x01ACTION tests a split message\r\n:emily!~e@localhost PRIVMSG papo-driver :\x01VERSION\x01\r\n:dan!~d@cloak PRIVMSG papo-driver :\x01PING 1731513600\x01\r\n:soju.example NOTICE papo-driver :\x01VERSION soju 0.8.2\x01\r\n").await;
    let notice = next_message(&mut events).await;
    assert_eq!(notice.kind, MessageKind::Notice);
    assert!(matches!(
        notice.conversation,
        crate::Conversation::Private(_)
    ));
    assert_eq!(notice.return_address(), "emily");
    let action = next_message(&mut events).await;
    assert_eq!(action.kind, MessageKind::Action);
    assert_eq!(
        action.text,
        "deploys bold and italic then both in red and hex"
    );
    assert_eq!(
        action.raw_text,
        "deploys \x02bold\x02 and \x1ditalic\x1d then \x02\x1dboth\x0f in \x0304red and \x04FF0000hex"
    );
    let spans = action.spans().collect::<Vec<_>>();
    assert_eq!(spans[1].text, "bold");
    assert!(spans[1].style.bold);
    assert_eq!(spans[2].style, crate::Style::default());
    assert!(spans[3].style.italics);
    assert_eq!(spans[4].style, crate::Style::default());
    assert!(spans[5].style.bold && spans[5].style.italics);
    assert_eq!(spans[6].style, crate::Style::default());
    assert_eq!(spans[7].style.fg, Some(crate::Color::Palette(4)));
    assert_eq!(spans[8].style.fg, Some(crate::Color::Hex(0xff_0000)));
    let action = next_message(&mut events).await;
    assert_eq!(action.kind, MessageKind::Action);
    assert_eq!(action.text, "tests a split message");
    assert_eq!(action.raw_text, "tests a split message");
    let version = events.next().await.unwrap();
    let version = version.as_ctcp().unwrap();
    assert_eq!(version.command, "VERSION");
    assert_eq!(version.params, None);
    assert!(!version.reply);
    let ping = events.next().await.unwrap();
    let ping = ping.as_ctcp().unwrap();
    assert_eq!(ping.command, "PING");
    assert_eq!(ping.params.as_deref(), Some("1731513600"));
    let reply = events.next().await.unwrap();
    let reply = reply.as_ctcp().unwrap();
    assert_eq!(reply.command, "VERSION");
    assert_eq!(reply.params.as_deref(), Some("soju 0.8.2"));
    assert!(reply.reply);

    server_line(&mut server, b":PAPO-DRIVER!p@localhost PRIVMSG #papo :sent\r\nBATCH +history42 chathistory #papo\r\n@batch=history42 :dan!d@localhost PRIVMSG #papo :earlier\r\n").await;
    assert_eq!(next_message(&mut events).await.origin, Origin::Live);
    assert_eq!(next_message(&mut events).await.origin, Origin::Replay);
    assert!(matches!(
        events.next().await,
        Some(Event::Warning(Warning::BatchAbandoned { .. }))
    ));

    let before = SystemTime::now();
    server_line(&mut server, b":dan!d@localhost PRIVMSG #papo :no time\r\n@time=invalid :dan!d@localhost PRIVMSG #papo :bad time\r\n").await;
    let absent = next_message(&mut events).await.time();
    let malformed = next_message(&mut events).await.time();
    let after = SystemTime::now();
    assert!((before..=after).contains(&absent));
    assert!((before..=after).contains(&malformed));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn wallops_is_a_server_notice() {
    let (mut server, _client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;

    server_line(
        &mut server,
        b":ergo.test WALLOPS :Network maintenance begins soon\r\n",
    )
    .await;
    let message = next_message(&mut events).await;
    assert_eq!(message.kind, MessageKind::Notice);
    assert_eq!(message.conversation, crate::Conversation::Server);
    assert_eq!(message.text, "Network maintenance begins soon");

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn registration_is_forwarded() {
    let (mut server, client, mut events, driver) = scripted(config());
    assert!(!client.state().is_connected());
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    assert!(client.state().is_connected());
    within(server.write_all(b":ergo.test CAP * LS :\r\n"))
        .await
        .unwrap();
    let mut cap_end = [0; 9];
    within(server.read_exact(&mut cap_end)).await.unwrap();
    assert_eq!(&cap_end, b"CAP END\r\n");
    within(server.write_all(b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 375 papo-driver :- ergo.test Message of the Day -\r\n:ergo.test 372 papo-driver :- Welcome to ErgoTest\r\n:ergo.test 376 papo-driver :End of MOTD\r\n"))
        .await
        .unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Motd(motd)) if motd.lines == ["- Welcome to ErgoTest"]
    ));
    assert!(matches!(events.next().await, Some(Event::Connected(_))));
    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
    assert!(!client.state().is_connected());
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_numeric_and_vendor_command_are_other_lines() {
    let (mut server, _client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;

    server_line(
        &mut server,
        b"@time=2026-10-20T12:00:00.000Z :ergo.test 777 papo-driver :Network-specific reply\r\n:irc.example ENCAP * REALHOST bob client.example\r\n:ergo.test PERSISTENCE STATUS DEFAULT ON\r\n",
    )
    .await;
    let numeric = events.next().await.unwrap();
    assert!(matches!(numeric, Event::Other(ref line) if line.numeric() == Some(777)));
    assert_eq!(
        numeric.as_other().unwrap().tag("time"),
        Some("2026-10-20T12:00:00.000Z")
    );
    assert!(matches!(
        events.next().await,
        Some(Event::Other(line))
            if line.command() == "ENCAP"
                && line.params() == ["*", "REALHOST", "bob", "client.example"]
    ));
    assert!(matches!(
        events.next().await,
        Some(Event::Other(line)) if line.command() == "PERSISTENCE"
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn list_modes_returns_entries_and_updates_joined_channel_state() {
    let (mut server, client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    server_line(&mut server, b":papo-driver!~papo@localhost JOIN #papo\r\n").await;

    let request = tokio::spawn(client.list_modes("#papo", crate::ListMode::Ban));
    client_line(&mut server, b"MODE #papo b\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 367 papo-driver #papo *!*@gateway.example oper 1761048000\r\n:ergo.test 368 papo-driver #papo :End of channel ban list\r\n",
    )
    .await;
    let entries = within(request).await.unwrap().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].mask, "*!*@gateway.example");
    assert_eq!(entries[0].actor.as_ref().unwrap().to_string(), "oper");
    assert_eq!(
        client
            .state()
            .channel("#papo")
            .unwrap()
            .list(crate::ListMode::Ban),
        Some(entries.as_slice())
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn list_modes_reports_the_entry_bound() {
    let (mut server, client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    let request = tokio::spawn(client.list_modes("#papo", crate::ListMode::Ban));
    client_line(&mut server, b"MODE #papo b\r\n").await;
    let rows = (0..1001).fold(String::new(), |mut rows, index| {
        write!(
            rows,
            ":ergo.test 367 papo-driver #papo *!*@host{index}.example oper 1761048000\r\n"
        )
        .unwrap();
        rows
    });
    server_line(&mut server, rows.as_bytes()).await;
    assert!(matches!(
        within(request).await.unwrap(),
        Err(Error::StateLimit(crate::StateLimit::ListEntries))
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn connection_failures_retry_up_to_the_configured_limit() {
    let calls = Arc::new(AtomicUsize::new(0));
    let connector: super::Connector = Arc::new({
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(transport_failure("scripted connection failure")) })
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
        super::PostConnect::default(),
        crate::Reconnect::exponential().max_attempts(2),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
        Box::new(|| 0),
    );
    let driver = tokio::spawn(driver);
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Connect(error),
            retry_in: Some(retry_in),
        })) if error.kind() == crate::ConnectErrorKind::Refused
            && retry_in == Duration::from_millis(800)
    ));
    assert!(matches!(
        client.privmsg("#papo", "first retry probe").await,
        Err(Error::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Connect(error),
            retry_in: Some(retry_in),
        })) if error.kind() == crate::ConnectErrorKind::Refused
            && retry_in == Duration::from_millis(800)
    ));
    tokio::time::advance(Duration::from_millis(300)).await;
    assert!(matches!(
        client.privmsg("#papo", "second retry probe").await,
        Err(Error::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Connect(error),
            retry_in: Some(retry_in),
        })) if error.kind() == crate::ConnectErrorKind::Refused
            && retry_in == Duration::from_millis(500)
    ));
    tokio::time::advance(Duration::from_millis(500)).await;
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Connect(error),
            retry_in: None,
        })) if error.kind() == crate::ConnectErrorKind::Refused
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Connect(_))
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn configuration_failures_are_terminal() {
    let mut failures = vec![Error::connect_unavailable("WebSocket")];
    #[cfg(tls)]
    failures.extend([
        Error::Connect(
            crate::transport::ConnectError::InvalidServerName("bad server name".into()).into(),
        ),
        Error::Connect(
            crate::transport::ConnectError::InvalidTlsConfiguration(
                "empty CA certificate document".into(),
            )
            .into(),
        ),
    ]);

    for failure in failures {
        let calls = Arc::new(AtomicUsize::new(0));
        let connector: super::Connector = Arc::new({
            let calls = Arc::clone(&calls);
            move |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                let failure = failure.clone();
                Box::pin(async move { Err(failure) })
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
            super::PostConnect::default(),
            Reconnect::exponential(),
            Box::new(papo_core::sts::MemoryStsStore::new()),
            connector,
        );
        let driver = tokio::spawn(driver);

        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected(disconnected)) if !disconnected.is_retrying()
        ));
        assert!(matches!(
            within(driver).await.unwrap(),
            Err(Error::Connect(_))
        ));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn registration_waiter_fails_after_one_connection_attempt() {
    let calls = Arc::new(AtomicUsize::new(0));
    let connector: super::Connector = Arc::new({
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(transport_failure("scripted connection failure")) })
        }
    });
    let (registration, waiting) = tokio::sync::oneshot::channel();
    let (_client, mut events, driver) = Driver::with_connector(
        Ok(ConnectionPlan {
            address: "irc://irc.example.net".parse().unwrap(),
            tls: TlsConfig::default(),
        }),
        Session::new(config()),
        super::DEFAULT_CHANNEL_CAPACITY,
        Some(registration),
        super::PostConnect::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);

    assert!(matches!(
        within(waiting).await.unwrap(),
        Err(Error::Connect(_))
    ));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if !disconnected.is_retrying()
    ));
    assert!(within(driver).await.unwrap().is_ok());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn registration_connection_loss_is_a_connect_error() {
    let (stream, mut server) = tokio::io::duplex(4096);
    let stream = Arc::new(Mutex::new(Some(Box::new(stream) as super::BoxConnection)));
    let calls = Arc::new(AtomicUsize::new(0));
    let connector: super::Connector = Arc::new({
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            let stream = stream.lock().unwrap().take().unwrap();
            Box::pin(async move { Ok(stream) })
        }
    });
    let (registration, waiting) = tokio::sync::oneshot::channel();
    let (_client, mut events, driver) = Driver::with_connector(
        Ok(ConnectionPlan {
            address: "irc://irc.example.net".parse().unwrap(),
            tls: TlsConfig::default(),
        }),
        Session::new(config()),
        super::DEFAULT_CHANNEL_CAPACITY,
        Some(registration),
        super::PostConnect::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    within(server.shutdown()).await.unwrap();

    assert!(matches!(
        within(waiting).await.unwrap(),
        Err(Error::Connect(error)) if error.kind() == crate::ConnectErrorKind::Other
    ));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Connect(error),
            retry_in: None,
        })) if error.kind() == crate::ConnectErrorKind::Other
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Connect(error)) if error.kind() == crate::ConnectErrorKind::Other
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn eof_reconnects_and_emits_reconnected() {
    let (first_stream, mut first_server) = tokio::io::duplex(4096);
    let (second_stream, mut second_server) = tokio::io::duplex(4096);
    let (third_stream, mut third_server) = tokio::io::duplex(4096);
    let streams = Arc::new(Mutex::new(VecDeque::from([
        Box::new(first_stream) as super::BoxConnection,
        Box::new(second_stream) as super::BoxConnection,
        Box::new(third_stream) as super::BoxConnection,
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
        super::PostConnect::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);

    let connected = complete_connect(&mut first_server, &mut events).await;
    assert_eq!(connected.nick, "papo-driver");
    assert_eq!(connected.welcome, "Welcome to the Ergo test network");
    assert!(!connected.reconnected);
    let join = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .join(test_channel("#papo"))
                .key("persistent-channel-key")
                .await
        }
    });
    client_line(&mut first_server, b"JOIN #papo persistent-channel-key\r\n").await;
    server_line(&mut first_server, b":papo-driver!papo@localhost JOIN #papo * :Papo Client\r\n:ergo.test 353 papo-driver = #papo :@papo-driver emily\r\n:ergo.test 366 papo-driver #papo :End of /NAMES list\r\n").await;
    within(join).await.unwrap().unwrap();
    client_line(&mut first_server, b"MODE #papo\r\n").await;
    client_line(&mut first_server, b"WHO #papo\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Joined(_))));
    assert!(matches!(events.next().await, Some(Event::ChannelSynced(_))));
    server_line(&mut first_server, b"@time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120000 :emily!~e@localhost PRIVMSG #papo :before disconnect\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    assert!(client.state().is_connected());
    within(first_server.shutdown()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if disconnected.is_retrying()
    ));
    assert!(!client.state().is_connected());

    tokio::time::advance(Duration::from_secs(2)).await;
    let connected = complete_connect(&mut second_server, &mut events).await;
    assert!(connected.reconnected);
    client_line(&mut second_server, b"JOIN #papo persistent-channel-key\r\n").await;
    server_line(
        &mut second_server,
        b":ergo.test 475 papo-driver #papo :Cannot join channel (+k)\r\n",
    )
    .await;
    assert!(matches!(
        events.next().await,
        Some(Event::AutoJoinFailed { channel, error })
            if channel == "#papo" && error.refusal() == crate::Refusal::BadChannelKey
    ));
    within(second_server.shutdown()).await.unwrap();
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));

    tokio::time::advance(Duration::from_secs(3)).await;
    welcome_burst(&mut third_server).await;
    server_line(&mut third_server, b":ergo.test CAP * LS :\r\n").await;
    client_line(&mut third_server, b"CAP END\r\n").await;
    server_line(&mut third_server, b":ergo.test 001 papo-driver :Welcome back\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Connected(_))));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), third_server.read_u8())
            .await
            .is_err()
    );
    assert!(client.state().is_connected());

    let quit = tokio::spawn(async move { client.quit("").await });
    client_line(&mut third_server, b"QUIT\r\n").await;
    within(third_server.shutdown()).await.unwrap();
    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
}

#[test]
#[allow(clippy::too_many_lines)]
fn rejoin_set_tracks_our_joins_parts_and_kicks() {
    let session = Session::new(config());
    let mut defaults = super::PostConnect {
        bot: false,
        channels: vec![super::Autojoin::new(
            "#staff",
            Some("private-channel-key".into()),
        )],
        parted_while_disconnected: vec![test_channel("#Papo"), test_channel("#rust")],
        history_on_reconnect: true,
        disconnected_at: None,
    };

    super::update_rejoin_channels(
        &session,
        &mut defaults,
        &Event::Joined(crate::event::Joined {
            msgid: None,
            time: SystemTime::UNIX_EPOCH,
            account: crate::Account::Unknown,
            realname: None,
            channel: test_channel("#papo"),
            actor: crate::__testing::user_source(test_nick("papo-driver"), None, None),
            nick: test_nick("papo-driver"),
            authorship: crate::Authorship::Own,
        }),
    );
    super::update_rejoin_channels(
        &session,
        &mut defaults,
        &Event::Joined(crate::event::Joined {
            msgid: None,
            time: SystemTime::UNIX_EPOCH,
            account: crate::Account::Unknown,
            realname: None,
            channel: test_channel("#visitors"),
            actor: crate::__testing::user_source(test_nick("alice"), None, None),
            nick: test_nick("alice"),
            authorship: crate::Authorship::Other,
        }),
    );
    assert_eq!(defaults.channels.len(), 2);

    super::update_rejoin_channels(
        &session,
        &mut defaults,
        &Event::Parted(crate::event::Parted {
            msgid: None,
            time: SystemTime::UNIX_EPOCH,
            account: None,
            channel: test_channel("#staff"),
            nick: test_nick("alice"),
            reason: Some("lunch".into()),
            authorship: crate::Authorship::Other,
        }),
    );
    super::update_rejoin_channels(
        &session,
        &mut defaults,
        &Event::Kicked(crate::event::Kicked {
            msgid: None,
            time: SystemTime::UNIX_EPOCH,
            account: None,
            channel: test_channel("#staff"),
            nick: test_nick("alice"),
            actor: crate::Source::from_proto(
                papo_proto::Source::User {
                    nick: "chanserv",
                    username: None,
                    host: None,
                },
                papo_proto::Casemapping::default(),
            ),
            authorship: crate::Authorship::Other,
            subject_authorship: crate::Authorship::Other,
            reason: Some("idle".into()),
        }),
    );
    assert_eq!(defaults.channels.len(), 2);

    super::update_rejoin_channels(
        &session,
        &mut defaults,
        &Event::Parted(crate::event::Parted {
            msgid: None,
            time: SystemTime::UNIX_EPOCH,
            account: None,
            channel: test_channel("#Papo"),
            nick: test_nick("papo-driver"),
            reason: None,
            authorship: crate::Authorship::Own,
        }),
    );
    assert_eq!(defaults.parted_while_disconnected, ["#rust"]);
    super::update_rejoin_channels(
        &session,
        &mut defaults,
        &Event::Kicked(crate::event::Kicked {
            msgid: None,
            time: SystemTime::UNIX_EPOCH,
            account: None,
            channel: test_channel("#staff"),
            nick: test_nick("papo-driver"),
            actor: crate::Source::from_proto(
                papo_proto::Source::User {
                    nick: "chanserv",
                    username: None,
                    host: None,
                },
                papo_proto::Casemapping::default(),
            ),
            authorship: crate::Authorship::Other,
            subject_authorship: crate::Authorship::Own,
            reason: Some("access removed".into()),
        }),
    );
    assert!(defaults.channels.is_empty());
}

#[test]
fn remembering_a_rejoin_replaces_only_the_same_channel() {
    let session = Session::new(config());
    let mut defaults = super::PostConnect {
        bot: false,
        channels: vec![
            super::Autojoin::new("#Papo", Some("old-channel-key".into())),
            super::Autojoin::new("#rust", None),
        ],
        parted_while_disconnected: Vec::new(),
        history_on_reconnect: true,
        disconnected_at: None,
    };

    super::remember_rejoin(
        &session,
        &mut defaults,
        super::Autojoin::new("#papo", Some("new-channel-key".into())),
    );

    assert_eq!(defaults.channels.len(), 2);
    assert_eq!(defaults.channels[0].channel, "#rust");
    assert_eq!(defaults.channels[1].channel, "#papo");
}

#[test]
fn only_bad_keys_remove_channels_from_the_rejoin_set() {
    let session = Session::new(config());
    let cases = [
        (
            ":ergo.test 471 papo-driver #papo :Cannot join channel (+l)",
            true,
        ),
        (
            ":ergo.test 473 papo-driver #papo :Cannot join channel (+i)",
            true,
        ),
        (
            ":irc.example.org 474 papo-driver #papo :Cannot join channel (+b)",
            true,
        ),
        (
            ":ergo.test 475 papo-driver #papo :Cannot join channel (+k)",
            false,
        ),
        (
            ":irc.example.org 477 papo-driver #papo :Cannot join channel (+r) - you need to be identified with services",
            true,
        ),
    ];

    for (line, retained) in cases {
        let mut defaults = super::PostConnect {
            channels: vec!["#Papo".into()],
            ..super::PostConnect::default()
        };
        let line = papo_proto::LineRef::parse(line).unwrap();
        super::drop_bad_rejoin_key(&session, &mut defaults, &line);
        assert_eq!(!defaults.channels.is_empty(), retained, "{line:?}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn write_timeout_resumes_before_the_next_line() {
    let now = crate::rt::now_mono();
    let mut session = Session::new(config().rate_limit(papo_core::RateLimit::unlimited()));
    session.handle_connected(now);
    while session.poll_transmit().is_some() {}
    session.handle_bytes(now, b":ergo.test CAP * LS :\r\n");
    assert_eq!(
        session.poll_transmit().as_deref(),
        Some(b"CAP END\r\n".as_slice())
    );
    session.handle_bytes(
        now,
        b":ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :No MOTD\r\n",
    );

    let channels = super::channels(8, &session);
    let mut pending_events = VecDeque::new();
    let mut pending_write = None;
    let mut requests = std::collections::BTreeMap::new();
    let mut transmits = std::collections::BTreeMap::new();
    let mut join_keys = std::collections::BTreeMap::new();
    let mut sync_waiters = Vec::new();
    let mut catchups = std::collections::BTreeMap::new();
    let mut registration = super::ConnectWait::default();
    let mut defaults = super::PostConnect {
        bot: false,
        channels: vec!["#papo".into(), "#rust".into()],
        parted_while_disconnected: Vec::new(),
        history_on_reconnect: false,
        disconnected_at: None,
    };
    let mut registered = false;
    let mut registered_at = None;
    let mut sts = super::StsActions::default();
    let mut sts_store = None;
    let mut stream = StallingWriter {
        written: Vec::new(),
        first_write: 8,
        stalled: false,
    };
    let deadline = crate::rt::now_mono() + Duration::from_millis(1);
    super::drain(
        &mut stream,
        &mut session,
        &mut pending_write,
        &channels.event_sink,
        &mut pending_events,
        &mut requests,
        &mut transmits,
        &mut join_keys,
        &mut sync_waiters,
        &mut catchups,
        &mut registration,
        &mut defaults,
        &mut registered,
        &mut registered_at,
        &mut sts,
        &mut sts_store,
        6697,
        false,
        false,
        Some(deadline),
    )
    .await
    .unwrap();
    assert_eq!(stream.written, b"JOIN #pa");
    assert_eq!(pending_write.as_ref().unwrap().offset, 8);

    stream.stalled = false;
    super::drain(
        &mut stream,
        &mut session,
        &mut pending_write,
        &channels.event_sink,
        &mut pending_events,
        &mut requests,
        &mut transmits,
        &mut join_keys,
        &mut sync_waiters,
        &mut catchups,
        &mut registration,
        &mut defaults,
        &mut registered,
        &mut registered_at,
        &mut sts,
        &mut sts_store,
        6697,
        false,
        false,
        None,
    )
    .await
    .unwrap();
    assert!(pending_write.is_none());
    assert_eq!(stream.written, b"JOIN #papo\r\nJOIN #rust\r\n");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn explicit_reconnect_redials_without_backoff() {
    let (first_stream, mut first_server) = tokio::io::duplex(4096);
    let (second_stream, mut second_server) = tokio::io::duplex(4096);
    let streams = Arc::new(Mutex::new(VecDeque::from([
        Box::new(first_stream) as super::BoxConnection,
        Box::new(second_stream) as super::BoxConnection,
    ])));
    let connection_times = Arc::new(Mutex::new(Vec::new()));
    let connector: super::Connector = Arc::new({
        let connection_times = Arc::clone(&connection_times);
        move |_| {
            connection_times
                .lock()
                .unwrap()
                .push(tokio::time::Instant::now());
            let stream = streams.lock().unwrap().pop_front().unwrap();
            Box::pin(async move { Ok(stream) })
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
        super::PostConnect::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
        Box::new(|| 0),
    );
    let driver = tokio::spawn(driver);
    complete_connect(&mut first_server, &mut events).await;

    let reconnect_requested_at = tokio::time::Instant::now();
    within(client.reconnect()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            retry_in: Some(Duration::ZERO),
            ..
        }))
    ));
    welcome_burst(&mut second_server).await;
    {
        let connection_times = connection_times.lock().unwrap();
        assert_eq!(connection_times.len(), 2);
        assert!(
            connection_times[1].duration_since(reconnect_requested_at) < Duration::from_millis(800)
        );
    }
    server_line(&mut second_server, b":ergo.test CAP * LS :\r\n").await;
    client_line(&mut second_server, b"CAP END\r\n").await;
    server_line(
        &mut second_server,
        b":ergo.test 001 papo-driver :Welcome back\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
    )
    .await;
    assert!(
        matches!(events.next().await, Some(Event::Connected(connected)) if connected.reconnected)
    );

    let quit = tokio::spawn(async move { client.quit("").await });
    client_line(&mut second_server, b"QUIT\r\n").await;
    within(second_server.shutdown()).await.unwrap();
    within(quit).await.unwrap().unwrap();
    within(driver).await.unwrap().unwrap();
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
#[allow(clippy::too_many_lines)]
async fn reconnect_history_pages_stop_at_the_target_limit() {
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
    let config = config().wanted_caps([
        "draft/chathistory",
        "labeled-response",
        "batch",
        "server-time",
    ]);
    let (_client, mut events, driver) = Driver::with_connector(
        Ok(ConnectionPlan {
            address: "irc://irc.example.net".parse().unwrap(),
            tls: TlsConfig::default(),
        }),
        Session::new(config),
        super::DEFAULT_CHANNEL_CAPACITY,
        None,
        super::PostConnect {
            bot: false,
            channels: vec!["#papo".into(), "#rust".into()],
            parted_while_disconnected: Vec::new(),
            history_on_reconnect: true,
            disconnected_at: None,
        },
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);

    complete_connect_history(&mut first_server, &mut events).await;
    client_line(
        &mut first_server,
        b"@label=0 JOIN #papo\r\n@label=1 JOIN #rust\r\n",
    )
    .await;
    server_line(&mut first_server, b"@time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120000 :emily!~e@localhost PRIVMSG #papo :before disconnect\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    within(first_server.shutdown()).await.unwrap();
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));
    tokio::time::advance(Duration::from_secs(2)).await;

    welcome_burst(&mut second_server).await;
    server_line(
        &mut second_server,
        b":ergo.test CAP * LS :draft/chathistory labeled-response batch server-time\r\n",
    )
    .await;
    client_line(
        &mut second_server,
        b"CAP REQ :draft/chathistory labeled-response batch server-time\r\n",
    )
    .await;
    server_line(
        &mut second_server,
        b":ergo.test CAP * ACK :draft/chathistory labeled-response batch server-time\r\n",
    )
    .await;
    client_line(&mut second_server, b"CAP END\r\n").await;
    server_line(&mut second_server, b":ergo.test 005 papo-driver CHATHISTORY=100 MSGREFTYPES=msgid,timestamp :supported\r\n:ergo.test 001 papo-driver :Welcome back\r\n").await;
    assert!(
        matches!(events.next().await, Some(Event::Connected(connected)) if connected.reconnected)
    );
    client_line(
        &mut second_server,
        b"@label=2 JOIN #papo\r\n@label=3 JOIN #rust\r\n",
    )
    .await;
    client_line(
        &mut second_server,
        b"@label=4 CHATHISTORY AFTER #papo msgid=RfcBTzyxY120000 100\r\n",
    )
    .await;
    let mut timestamp_request = Vec::new();
    loop {
        let byte = within(second_server.read_u8()).await.unwrap();
        timestamp_request.push(byte);
        if byte == b'\n' {
            break;
        }
    }
    let timestamp_request = String::from_utf8(timestamp_request).unwrap();
    assert!(timestamp_request.starts_with("@label=5 CHATHISTORY AFTER #rust timestamp="));
    assert!(timestamp_request.ends_with(" 100\r\n"));
    let mut first_page = String::from(
        "@label=4 :ergo.test BATCH +reply0 labeled-response\r\n@batch=reply0 :ergo.test BATCH +history0 chathistory #papo\r\n",
    );
    for index in 0..=950 {
        use std::fmt::Write as _;
        writeln!(
            first_page,
            "@batch=history0;time=2026-10-15T12:01:00.000Z;msgid=RfcBTzyxY12{index:04} :emily!~e@localhost PRIVMSG #papo :missed {index}\r"
        )
        .unwrap();
    }
    first_page.push_str("@batch=reply0 :ergo.test BATCH -history0\r\n:ergo.test BATCH -reply0\r\n");
    server_line(&mut second_server, first_page.as_bytes()).await;
    client_line(
        &mut second_server,
        b"@label=6 CHATHISTORY AFTER #papo msgid=RfcBTzyxY120950 50\r\n",
    )
    .await;
    let mut second_page = String::from(
        "@label=6 :ergo.test BATCH +reply2 labeled-response\r\n@batch=reply2 :ergo.test BATCH +history2 chathistory #papo\r\n",
    );
    for index in 950..=1_000 {
        use std::fmt::Write as _;
        writeln!(
            second_page,
            "@batch=history2;time=2026-10-15T12:02:00.000Z;msgid=RfcBTzyxY12{index:04} :emily!~e@localhost PRIVMSG #papo :missed {index}\r"
        )
        .unwrap();
    }
    second_page
        .push_str("@batch=reply2 :ergo.test BATCH -history2\r\n:ergo.test BATCH -reply2\r\n");
    server_line(&mut second_server, second_page.as_bytes()).await;

    let Some(Event::History(history)) = events.next().await else {
        panic!("expected aggregate history event")
    };
    assert!(
        matches!(&history.conversation, crate::Conversation::Channel(channel) if channel == "#papo")
    );
    assert_eq!(history.messages().count(), 1_000);
    assert_eq!(
        history
            .messages()
            .next()
            .map(|message| message.text.as_str()),
        Some("missed 1")
    );
    assert_eq!(
        history
            .messages()
            .last()
            .map(|message| message.text.as_str()),
        Some("missed 1000")
    );
    assert!(
        history
            .messages()
            .all(|message| message.origin == Origin::History && !message.is_live())
    );
    assert!(!history.end);
    abort_task(driver).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reconnect_history_advances_past_the_previous_catchup() {
    let (first_stream, mut first_server) = tokio::io::duplex(4096);
    let (second_stream, mut second_server) = tokio::io::duplex(4096);
    let (third_stream, mut third_server) = tokio::io::duplex(4096);
    let streams = Arc::new(Mutex::new(VecDeque::from([
        Box::new(first_stream) as super::BoxConnection,
        Box::new(second_stream) as super::BoxConnection,
        Box::new(third_stream) as super::BoxConnection,
    ])));
    let connector: super::Connector = Arc::new(move |_| {
        let stream = streams.lock().unwrap().pop_front().unwrap();
        Box::pin(async move { Ok(stream) })
    });
    let config = config().wanted_caps([
        "draft/chathistory",
        "labeled-response",
        "batch",
        "server-time",
    ]);
    let (_client, mut events, driver) = Driver::with_connector(
        Ok(ConnectionPlan {
            address: "irc://irc.example.net".parse().unwrap(),
            tls: TlsConfig::default(),
        }),
        Session::new(config),
        super::DEFAULT_CHANNEL_CAPACITY,
        None,
        super::PostConnect {
            bot: false,
            channels: vec!["#papo".into()],
            parted_while_disconnected: Vec::new(),
            history_on_reconnect: true,
            disconnected_at: None,
        },
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);

    complete_connect_history(&mut first_server, &mut events).await;
    client_line(&mut first_server, b"@label=0 JOIN #papo\r\n").await;
    server_line(&mut first_server, b"@time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120000 :emily!~e@localhost PRIVMSG #papo :before disconnect\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    within(first_server.shutdown()).await.unwrap();
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));
    tokio::time::advance(Duration::from_secs(2)).await;

    complete_connect_history(&mut second_server, &mut events).await;
    client_line(&mut second_server, b"@label=1 JOIN #papo\r\n").await;
    client_line(
        &mut second_server,
        b"@label=2 CHATHISTORY AFTER #papo msgid=RfcBTzyxY120000 100\r\n",
    )
    .await;
    server_line(&mut second_server, b"@label=2 :ergo.test BATCH +reply0 labeled-response\r\n@batch=reply0;draft/chathistory-end :ergo.test BATCH +history0 chathistory #papo\r\n@batch=history0;time=2026-10-15T12:01:00.000Z;msgid=RfcBTzyxY120001 :emily!~e@localhost PRIVMSG #papo :missed once\r\n@batch=reply0 :ergo.test BATCH -history0\r\n:ergo.test BATCH -reply0\r\n").await;
    let Some(Event::History(history)) = events.next().await else {
        panic!("expected first catch-up")
    };
    assert_eq!(
        history
            .messages()
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>(),
        ["missed once"]
    );
    within(second_server.shutdown()).await.unwrap();
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));
    tokio::time::advance(Duration::from_secs(2)).await;

    complete_connect_history(&mut third_server, &mut events).await;
    client_line(&mut third_server, b"@label=3 JOIN #papo\r\n").await;
    client_line(
        &mut third_server,
        b"@label=4 CHATHISTORY AFTER #papo msgid=RfcBTzyxY120001 100\r\n",
    )
    .await;
    server_line(&mut third_server, b"@label=4 :ergo.test BATCH +reply1 labeled-response\r\n@batch=reply1;draft/chathistory-end :ergo.test BATCH +history1 chathistory #papo\r\n@batch=reply1 :ergo.test BATCH -history1\r\n:ergo.test BATCH -reply1\r\n").await;
    let Some(Event::History(history)) = events.next().await else {
        panic!("expected empty second catch-up")
    };
    assert!(history.items.is_empty());
    abort_task(driver).await;
}

#[test]
fn reconnect_history_stops_at_the_literal_per_target_limit() {
    for count in [1_000, 1_001] {
        let mut catchup = super::Catchup {
            target: "#papo".into(),
            items: Vec::new(),
            seen: std::collections::HashSet::new(),
        };
        let next = super::extend_catchup(
            &mut catchup,
            (0..count).map(|index| {
                papo_core::HistoryItem::Message(papo_core::HistoryLine {
                    line: Line::privmsg("#papo", format!("backlog message {index}"))
                        .with_tag("msgid", format!("RfcBTzyxY12{index:04}")),
                    context: false,
                    authorship: papo_core::Authorship::Other,
                    subject_authorship: papo_core::Authorship::Unknown,
                })
            }),
        );
        assert_eq!(catchup.items.len(), 1_000);
        assert_eq!(next.as_deref(), Some("RfcBTzyxY120999"));
    }
}

#[cfg(feature = "unstable")]
#[test]
fn persistence_gates_rejoin_and_history_fetch() {
    for (effective, expected) in [(false, true), (true, false)] {
        let now = std::time::Instant::now();
        let mut session = Session::new(
            config()
                .enable(papo_core::Feature::Persistence)
                .wanted_caps(["draft/persistence"]),
        );
        session.handle_connected(now);
        while session.poll_transmit().is_some() {}
        session.handle_bytes(now, b":ergo.test CAP * LS :draft/persistence\r\n");
        while session.poll_transmit().is_some() {}
        session.handle_bytes(
            now,
            b":ergo.test CAP * ACK :draft/persistence\r\n:ergo.test PERSISTENCE STATUS DEFAULT ",
        );
        session.handle_bytes(now, if effective { b"ON\r\n" } else { b"OFF\r\n" });
        assert_eq!(super::should_rejoin(&session), expected);
        assert_eq!(super::should_fetch_history(&session), expected);
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn actions_fail_while_disconnected_and_are_not_sent_after_reconnect() {
    let (client_stream, mut server) = tokio::io::duplex(4096);
    let results = Arc::new(Mutex::new(VecDeque::from([
        Err(transport_failure("scripted connection failure")),
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
        super::PostConnect {
            channels: vec!["#papo".into()],
            ..super::PostConnect::default()
        },
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if disconnected.is_retrying()
    ));

    assert!(matches!(
        within(client.send_raw(crate::Line::new("KILL", ["emily", "connection reset"],))).await,
        Err(disconnected!())
    ));
    assert!(matches!(
        within(client.privmsg("#papo", "lost during reconnect")).await,
        Err(disconnected!())
    ));
    assert!(matches!(
        within(client.whois(test_nick("emily"))).await,
        Err(disconnected!())
    ));
    assert!(matches!(
        within(client.part(test_channel("#papo"), "leaving while offline")).await,
        Err(disconnected!())
    ));

    tokio::time::advance(Duration::from_secs(2)).await;
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
    client_line(&mut server, b"CAP END\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Connected(_))));
    server_line(
        &mut server,
        b":papo-driver!papo@localhost JOIN #papo * :Papo Client\r\n",
    )
    .await;
    client_line(&mut server, b"PART #papo\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Joined(_))));
    server_line(
        &mut server,
        b":papo-driver!papo@localhost PART #papo :Leaving\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Parted(_))));
    let quit = tokio::spawn(async move { client.quit("").await });
    client_line(&mut server, b"QUIT\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn restored_offline_part_is_not_rejoined_after_an_interrupted_part() {
    let (first_stream, mut first_server) = tokio::io::duplex(4096);
    let (second_stream, mut second_server) = tokio::io::duplex(4096);
    let results = Arc::new(Mutex::new(VecDeque::from([
        Err(transport_failure("scripted connection failure")),
        Ok(Box::new(first_stream) as super::BoxConnection),
        Ok(Box::new(second_stream) as super::BoxConnection),
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
        super::PostConnect {
            channels: vec!["#papo".into(), "#rust".into()],
            ..super::PostConnect::default()
        },
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if disconnected.is_retrying()
    ));
    assert!(matches!(
        within(client.part(test_channel("#papo"), "leaving while offline")).await,
        Err(disconnected!())
    ));
    assert!(matches!(
        within(client.part(test_channel("#rust"), "leaving while offline")).await,
        Err(disconnected!())
    ));

    within(client.reconnect()).await.unwrap();
    complete_connect(&mut first_server, &mut events).await;
    server_line(
        &mut first_server,
        b":papo-driver!papo@localhost JOIN #papo * :Papo Client\r\n",
    )
    .await;
    client_line(&mut first_server, b"PART #papo\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Joined(_))));
    server_line(
        &mut first_server,
        b":papo-driver!papo@localhost JOIN #rust * :Papo Client\r\n",
    )
    .await;
    client_line(&mut first_server, b"PART #rust\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Joined(_))));
    within(first_server.shutdown()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if disconnected.is_retrying()
    ));

    within(client.reconnect()).await.unwrap();
    complete_connect(&mut second_server, &mut events).await;
    let quit = tokio::spawn(async move { client.quit("").await });
    client_line(&mut second_server, b"QUIT\r\n").await;
    within(second_server.shutdown()).await.unwrap();
    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn quit_while_disconnected_stops_without_another_attempt() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let connector: super::Connector = Arc::new({
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(transport_failure("scripted connection failure")) })
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
        super::PostConnect::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if disconnected.is_retrying()
    ));
    within(client.reconnect()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if disconnected.is_retrying()
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    within(client.quit("cancel reconnect")).await.unwrap();
    assert!(within(driver).await.unwrap().is_ok());
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Requested {
                quit_unacknowledged: false,
            },
            retry_in: None,
        }))
    ));
    assert!(events.next().await.is_none());
    tokio::time::advance(Duration::from_secs(10)).await;
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn stalled_consumer_does_not_block_quit_after_connection_loss() {
    let (stream, mut server) = tokio::io::duplex(4096);
    let stream = Arc::new(Mutex::new(Some(Box::new(stream) as super::BoxConnection)));
    let connector: super::Connector = Arc::new(move |_| {
        let stream = stream.lock().unwrap().take().unwrap();
        Box::pin(async move { Ok(stream) })
    });
    let (client, mut events, driver) = Driver::with_connector(
        Ok(ConnectionPlan {
            address: "irc://irc.example.net".parse().unwrap(),
            tls: TlsConfig::default(),
        }),
        Session::new(config()),
        1,
        None,
        super::PostConnect::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test NOTICE papo-driver :maintenance soon\r\n",
    )
    .await;
    while events.receiver.len() != 1 {
        tokio::task::yield_now().await;
    }

    within(server.shutdown()).await.unwrap();
    while client.state().is_connected() {
        tokio::task::yield_now().await;
    }
    within(client.quit("")).await.unwrap();
    assert!(within(driver).await.unwrap().is_ok());
}

fn failing_driver_with_one_event_slot() -> (super::Client, super::Events, Driver, Arc<AtomicUsize>)
{
    let calls = Arc::new(AtomicUsize::new(0));
    let connector: super::Connector = Arc::new({
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(transport_failure("scripted connection failure")) })
        }
    });
    let (client, events, driver) = Driver::with_connector_and_entropy(
        Ok(ConnectionPlan {
            address: "irc://irc.example.net".parse().unwrap(),
            tls: TlsConfig::default(),
        }),
        Session::new(config()),
        1,
        None,
        super::PostConnect::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
        Box::new(|| 0),
    );
    (client, events, driver, calls)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn stalled_consumer_does_not_block_quit_after_connect_failure() {
    let (client, events, driver, calls) = failing_driver_with_one_event_slot();
    let driver = tokio::spawn(driver);
    while events.receiver.len() != 1 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(1)).await;
    while calls.load(Ordering::Relaxed) != 2 {
        tokio::task::yield_now().await;
    }

    within(client.quit("")).await.unwrap();
    assert!(within(driver).await.unwrap().is_ok());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn failed_connect_events_obey_the_overflow_bound() {
    let (_client, mut events, driver, calls) = failing_driver_with_one_event_slot();
    let driver = tokio::spawn(driver);
    while events.receiver.len() != 1 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(1)).await;
    while calls.load(Ordering::Relaxed) != 2 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(2)).await;

    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::SlowConsumer,
            retry_in: None,
        }))
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn failed_connects_back_off_until_the_attempt_limit() {
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let connector: super::Connector = Arc::new({
        let attempts = Arc::clone(&attempts);
        move |_| {
            attempts.lock().unwrap().push(tokio::time::Instant::now());
            Box::pin(async { Err(transport_failure("scripted connection failure")) })
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
        super::PostConnect::default(),
        Reconnect::exponential().max_attempts(3),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
        Box::new(|| 0),
    );
    let driver = tokio::spawn(driver);

    for expected in [true, true, false] {
        assert!(matches!(
            events.next().await,
            Some(Event::Disconnected(disconnected)) if disconnected.is_retrying() == expected
        ));
    }
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Connect(_))
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
        Err(transport_failure("first scripted failure")),
        Err(transport_failure("second scripted failure")),
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
        super::PostConnect::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
        Box::new(|| 0),
    );
    let driver = tokio::spawn(driver);
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));

    welcome_burst(&mut stable_server).await;
    server_line(&mut stable_server, b":ergo.test CAP * LS :\r\n").await;
    client_line(&mut stable_server, b"CAP END\r\n").await;
    server_line(
        &mut stable_server,
        b":ergo.test 001 papo-driver :Welcome back\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Connected(_))));
    tokio::time::advance(Duration::from_secs(60)).await;
    let disconnected_at = tokio::time::Instant::now();
    within(stable_server.shutdown()).await.unwrap();
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));

    welcome_burst(&mut next_server).await;
    let delay = {
        let attempts = attempts.lock().unwrap();
        assert_eq!(attempts.len(), 4);
        attempts[3] - disconnected_at
    };
    assert_eq!(delay, Duration::from_millis(800));
    let quit = tokio::spawn(async move { client.quit("").await });
    client_line(&mut next_server, b"QUIT\r\n").await;
    within(next_server.shutdown()).await.unwrap();
    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
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
        papo_core::sts::StsPolicy::new(
            "irc.example.net",
            7443,
            std::time::SystemTime::now() + Duration::from_secs(3600),
            Duration::from_secs(3600),
        ),
    );
    let (client, _events, driver) = Driver::with_connector(
        Ok(ConnectionPlan {
            address: "irc://irc.example.net".parse().unwrap(),
            tls: TlsConfig::default(),
        }),
        Session::new(config()),
        super::DEFAULT_CHANNEL_CAPACITY,
        None,
        super::PostConnect::default(),
        Reconnect::exponential(),
        Box::new(store),
        connector,
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    let quit = tokio::spawn(async move { client.quit("").await });
    client_line(&mut server, b"QUIT\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn transport_failure_under_stored_sts_retries() {
    let calls = Arc::new(AtomicUsize::new(0));
    let connector: super::Connector = Arc::new({
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async {
                Err(Error::Connect(crate::ConnectError::new(
                    crate::ConnectErrorKind::Refused,
                    io::Error::new(io::ErrorKind::ConnectionRefused, "TLS endpoint refused"),
                )))
            })
        }
    });
    let mut store = papo_core::sts::MemoryStsStore::new();
    papo_core::sts::StsStore::put(
        &mut store,
        papo_core::sts::StsPolicy::new(
            "irc.example.net",
            6697,
            SystemTime::now() + Duration::from_secs(3600),
            Duration::from_secs(3600),
        ),
    );
    let (_client, mut events, driver) = Driver::with_connector_and_entropy(
        Ok(ConnectionPlan {
            address: "irc://irc.example.net".parse().unwrap(),
            tls: TlsConfig::default(),
        }),
        Session::new(config()),
        super::DEFAULT_CHANNEL_CAPACITY,
        None,
        super::PostConnect::default(),
        Reconnect::exponential().max_attempts(2),
        Box::new(store),
        connector,
        Box::new(|| 0),
    );
    let driver = tokio::spawn(driver);

    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Connect(error),
            retry_in: Some(retry_in),
        })) if error.kind() == crate::ConnectErrorKind::Refused
            && retry_in == Duration::from_millis(800)
    ));
    tokio::time::advance(Duration::from_millis(800)).await;
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Connect(_))
    ));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if !disconnected.is_retrying()
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn eof_emits_disconnected_and_returns_an_error() {
    let (mut server, _client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::ConnectionLost { .. },
            retry_in: None,
        }))
    ));
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_error_does_not_replace_eof() {
    let (mut server, _client, mut events, driver) = scripted_with_failures(false);
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::ConnectionLost { .. },
            retry_in: None,
        }))
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::ConnectionLost { .. },
            ..
        }))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn read_error_is_connection_loss() {
    let (mut server, _client, mut events, driver) = scripted_with_failures(true);
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    let Some(Event::Disconnected(disconnected)) = events.next().await else {
        panic!("expected connection loss")
    };
    assert_eq!(
        disconnected.to_string(),
        "the connection was lost: original read failure"
    );
    assert_eq!(
        std::error::Error::source(&disconnected).map(ToString::to_string),
        Some("the connection was lost: original read failure".into())
    );
    assert_eq!(disconnected.retry_in, None);
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::ConnectionLost { .. },
            ..
        }))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn overlong_line_becomes_a_terminal_disconnect() {
    let (mut server, _client, mut events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    within(server.write_all(&vec![b'x'; 8704])).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Protocol { message },
            retry_in: None,
        })) if message == "incoming IRC line exceeds 8703 bytes"
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_line_preserves_exact_bytes() {
    let (mut server, _client, mut events, driver) = scripted(config().decoding(Decoding::Utf8));
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    within(server.write_all(b":ergo.test NOTICE * :caf\xe9\r\n"))
        .await
        .unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Warning(Warning::Malformed { line }))
            if line == b":ergo.test NOTICE * :caf\xe9"
    ));
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn sasl_nak_forwards_authentication_failure() {
    let settings = config()
        .wanted_caps(["sasl"])
        .sasl(Sasl::plain("alice", "correct horse battery staple").optional());
    let (mut server, _client, mut events, driver) = scripted(settings);
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :sasl=PLAIN\r\n").await;
    client_line(&mut server, b"CAP REQ :sasl\r\n").await;
    server_line(&mut server, b":ergo.test CAP * NAK :sasl\r\n").await;
    assert!(matches!(
        events.next().await,
        Some(Event::Warning(Warning::AuthenticationFailed { .. }))
    ));
    client_line(&mut server, b"CAP END\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn required_sasl_failure_rejects_connect_waiter() {
    let settings = config()
        .wanted_caps(["sasl"])
        .sasl(Sasl::plain("alice", "correct horse battery staple"));
    let (client_stream, mut server) = tokio::io::duplex(4096);
    let (registration, waiter) = tokio::sync::oneshot::channel();
    let (_client, _events, driver) = Driver::with_stream_and_connect(
        client_stream,
        Session::new(settings),
        super::DEFAULT_CHANNEL_CAPACITY,
        Some(registration),
        super::PostConnect::default(),
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :sasl=PLAIN\r\n").await;
    client_line(&mut server, b"CAP REQ :sasl\r\n").await;
    server_line(&mut server, b":ergo.test CAP * ACK :sasl\r\n").await;
    client_line(&mut server, b"AUTHENTICATE PLAIN\r\n").await;
    server_line(&mut server, b"AUTHENTICATE +\r\n").await;
    client_line(
        &mut server,
        b"AUTHENTICATE AGFsaWNlAGNvcnJlY3QgaG9yc2UgYmF0dGVyeSBzdGFwbGU=\r\n",
    )
    .await;
    server_line(
        &mut server,
        b":ergo.test 904 alice :SASL authentication failed\r\n",
    )
    .await;

    assert!(matches!(
        within(waiter).await.unwrap(),
        Err(Error::Connect(error))
            if error.kind() == crate::ConnectErrorKind::Sasl
                && std::error::Error::source(&error)
                    .is_some_and(|source| source.to_string().contains("SASL"))
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Connect(error)) if error.kind() == crate::ConnectErrorKind::Sasl
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn account_required_cap_rejects_registration_without_sasl() {
    let (client_stream, mut server) = tokio::io::duplex(4096);
    let (registration, waiter) = tokio::sync::oneshot::channel();
    let (_client, _events, driver) = Driver::with_stream_and_connect(
        client_stream,
        Session::new(config()),
        super::DEFAULT_CHANNEL_CAPACITY,
        Some(registration),
        super::PostConnect::default(),
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(
        &mut server,
        b":soju.example CAP * LS :soju.im/account-required sasl=PLAIN\r\n",
    )
    .await;

    assert!(matches!(
        within(waiter).await.unwrap(),
        Err(Error::Connect(error))
            if error.kind() == crate::ConnectErrorKind::AccountRequired
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Connect(error))
            if error.kind() == crate::ConnectErrorKind::AccountRequired
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn terminal_numerics_have_specific_connect_errors() {
    for (line, expected, diagnostic) in [
        (
            ":ergo.test 464 * :Password incorrect\r\n",
            crate::ConnectErrorKind::PasswordIncorrect,
            "Password incorrect",
        ),
        (
            ":ergo.test 465 papo-driver :You are banned from this server\r\n",
            crate::ConnectErrorKind::Banned,
            "You are banned from this server",
        ),
    ] {
        let (client_stream, mut server) = tokio::io::duplex(4096);
        let (registration, waiter) = tokio::sync::oneshot::channel();
        let (_client, _events, driver) = Driver::with_stream_and_connect(
            client_stream,
            Session::new(config()),
            super::DEFAULT_CHANNEL_CAPACITY,
            Some(registration),
            super::PostConnect::default(),
        );
        let driver = tokio::spawn(driver);
        welcome_burst(&mut server).await;
        server_line(&mut server, line.as_bytes()).await;

        assert!(matches!(
            within(waiter).await.unwrap(),
            Err(Error::Connect(error))
                if error.kind() == expected
                    && std::error::Error::source(&error)
                        .is_some_and(|source| source.to_string() == diagnostic)
        ));
        assert!(matches!(
            within(driver).await.unwrap(),
            Err(Error::Connect(error)) if error.kind() == expected
        ));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn server_error_before_welcome_is_a_connection_failure() {
    let settings = config().webirc(papo_core::WebIrc {
        password: papo_core::Secret::from("gateway-password"),
        gateway: "papo-gateway".into(),
        hostname: None,
        ip: "192.0.2.42".into(),
        options: Vec::new(),
    });
    let (client_stream, mut server) = tokio::io::duplex(4096);
    let (registration, waiter) = tokio::sync::oneshot::channel();
    let (_client, mut events, driver) = Driver::with_stream_and_connect(
        client_stream,
        Session::new(settings),
        super::DEFAULT_CHANNEL_CAPACITY,
        Some(registration),
        super::PostConnect::default(),
    );
    let driver = tokio::spawn(driver);
    client_line(
        &mut server,
        b"WEBIRC gateway-password papo-gateway 192.0.2.42 192.0.2.42\r\n",
    )
    .await;
    welcome_burst(&mut server).await;
    server_line(
        &mut server,
        b"ERROR :Too many clients from your network\r\n",
    )
    .await;

    let Some(Event::Disconnected(disconnected)) = events.next().await else {
        panic!("expected connection failure")
    };
    assert!(matches!(
        &disconnected.cause,
        crate::DisconnectCause::Connect(error)
            if error.kind() == crate::ConnectErrorKind::Server
                && std::error::Error::source(error).is_some_and(|source| {
                    source.to_string() == "Too many clients from your network"
                })
    ));
    assert!(matches!(
        within(waiter).await.unwrap(),
        Err(Error::Connect(error)) if error.kind() == crate::ConnectErrorKind::Server
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Connect(error)) if error.kind() == crate::ConnectErrorKind::Server
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn webirc_rejection_is_a_specific_connection_failure() {
    let settings = config().webirc(papo_core::WebIrc {
        password: papo_core::Secret::from("gateway-password"),
        gateway: "papo-gateway".into(),
        hostname: None,
        ip: "192.0.2.42".into(),
        options: Vec::new(),
    });
    let (client_stream, mut server) = tokio::io::duplex(4096);
    let (registration, waiter) = tokio::sync::oneshot::channel();
    let (_client, mut events, driver) = Driver::with_stream_and_connect(
        client_stream,
        Session::new(settings),
        super::DEFAULT_CHANNEL_CAPACITY,
        Some(registration),
        super::PostConnect::default(),
    );
    let driver = tokio::spawn(driver);
    client_line(
        &mut server,
        b"WEBIRC gateway-password papo-gateway 192.0.2.42 192.0.2.42\r\n",
    )
    .await;
    welcome_burst(&mut server).await;
    server_line(
        &mut server,
        b"ERROR :WEBIRC command is not usable from your address or incorrect password given\r\n",
    )
    .await;

    let Some(Event::Disconnected(disconnected)) = events.next().await else {
        panic!("expected WEBIRC connection failure")
    };
    assert!(matches!(
        &disconnected.cause,
        crate::DisconnectCause::Connect(error)
            if error.kind() == crate::ConnectErrorKind::WebIrcRejected
                && std::error::Error::source(error).is_some_and(|source| {
                    source.to_string()
                        == "WEBIRC command is not usable from your address or incorrect password given"
                })
    ));
    assert!(matches!(
        within(waiter).await.unwrap(),
        Err(Error::Connect(error))
            if error.kind() == crate::ConnectErrorKind::WebIrcRejected
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Connect(error))
            if error.kind() == crate::ConnectErrorKind::WebIrcRejected
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn nick_candidates_exhaustion_ends_login_immediately() {
    let settings = config()
        .alt_nicks(["papo-away"])
        .welcome_timeout(Duration::from_secs(23));
    let (client_stream, mut server) = tokio::io::duplex(4096);
    let (registration, waiter) = tokio::sync::oneshot::channel();
    let (_client, mut events, driver) = Driver::with_stream_and_connect(
        client_stream,
        Session::new(settings),
        super::DEFAULT_CHANNEL_CAPACITY,
        Some(registration),
        super::PostConnect::default(),
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(
        &mut server,
        b":ergo.test 005 * NICKLEN=1 :are supported by this server\r\n",
    )
    .await;
    server_line(
        &mut server,
        b":ergo.test 400 * NICK :Nickname is already in use\r\n",
    )
    .await;
    client_line(&mut server, b"NICK papo-away\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 400 * NICK :Nickname is already in use\r\n",
    )
    .await;
    client_line(&mut server, b"NICK _\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 400 * NICK :Nickname is already in use\r\n",
    )
    .await;

    let Some(Event::Disconnected(disconnected)) = events.next().await else {
        panic!("expected nick exhaustion")
    };
    assert!(matches!(
        &disconnected.cause,
        crate::DisconnectCause::Connect(error)
            if error.kind() == crate::ConnectErrorKind::Server
                && std::error::Error::source(error)
                    .is_some_and(|source| source.to_string() == "nick candidates exhausted")
    ));
    assert!(matches!(
        within(waiter).await.unwrap(),
        Err(Error::Connect(error))
            if error.kind() == crate::ConnectErrorKind::Server
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Connect(error))
            if error.kind() == crate::ConnectErrorKind::Server
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn required_sasl_failure_does_not_reconnect() {
    let settings = config()
        .wanted_caps(["sasl"])
        .sasl(Sasl::plain("alice", "correct horse battery staple"));
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
        super::PostConnect::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :sasl=PLAIN\r\n").await;
    client_line(&mut server, b"CAP REQ :sasl\r\n").await;
    server_line(&mut server, b":ergo.test CAP * NAK :sasl\r\n").await;

    let Some(Event::Disconnected(disconnected)) = events.next().await else {
        panic!("expected login disconnect")
    };
    assert!(matches!(
        &disconnected,
        crate::Disconnected {
            cause: crate::DisconnectCause::Connect(error),
            retry_in: None,
        } if error.kind() == crate::ConnectErrorKind::Sasl
    ));
    assert!(matches!(
        std::error::Error::source(&disconnected),
        Some(source) if source.to_string().contains("SASL")
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Connect(error)) if error.kind() == crate::ConnectErrorKind::Sasl
    ));
    tokio::time::advance(Duration::from_secs(10)).await;
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[cfg(tls)]
#[test]
fn certificate_verification_failure_is_fatal() {
    let error = Error::Connect(
        crate::transport::ConnectError::Tls(rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownIssuer,
        ))
        .into(),
    );
    assert!(super::is_fatal_failure(&error, None));
    let Event::Disconnected(disconnected) =
        Event::Disconnected(super::disconnected_event(&error, None))
    else {
        unreachable!()
    };
    let crate::DisconnectCause::Connect(connect) = &disconnected.cause else {
        panic!("expected the certificate connection error")
    };
    assert_eq!(connect.kind(), crate::ConnectErrorKind::CertificateRejected);
    let source = std::error::Error::source(&disconnected).unwrap();
    assert!(
        source
            .source()
            .unwrap()
            .to_string()
            .contains("UnknownIssuer")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn successful_registration_preserves_earlier_events() {
    let (client_stream, mut server) = tokio::io::duplex(4096);
    let (registration, waiter) = tokio::sync::oneshot::channel();
    let (_client, mut events, driver) = Driver::with_stream_and_connect(
        client_stream,
        Session::new(config()),
        super::DEFAULT_CHANNEL_CAPACITY,
        Some(registration),
        super::PostConnect::default(),
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
    client_line(&mut server, b"CAP END\r\n").await;
    server_line(
        &mut server,
        b":ergo.test NOTICE * :*** Looking up your hostname...\r\n:ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 376 papo-driver :End of MOTD\r\n",
    )
    .await;

    assert!(within(waiter).await.unwrap().is_ok());
    assert!(matches!(
        events.next().await,
        Some(Event::Message(message)) if message.kind == MessageKind::Notice
    ));
    assert!(matches!(events.next().await, Some(Event::Connected(_))));
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn verification_challenge_resolves_registration_before_welcome() {
    let (client_stream, mut server) = tokio::io::duplex(4096);
    let (registration, waiter) = tokio::sync::oneshot::channel();
    let config = config()
        .wanted_caps(["draft/account-registration"])
        .create_account_on_connect(
            Some("alice"),
            Some("alice@example.org"),
            "correct horse battery staple",
        );
    let (_client, mut events, driver) = Driver::with_stream_and_connect(
        client_stream,
        Session::new(config),
        super::DEFAULT_CHANNEL_CAPACITY,
        Some(registration),
        super::PostConnect::default(),
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(
        &mut server,
        b":ergo.test CAP * LS :draft/account-registration=before-connect\r\n",
    )
    .await;
    client_line(&mut server, b"CAP REQ :draft/account-registration\r\n").await;
    server_line(
        &mut server,
        b":ergo.test CAP * ACK :draft/account-registration\r\n",
    )
    .await;
    client_line(
        &mut server,
        b"REGISTER alice alice@example.org :correct horse battery staple\r\n",
    )
    .await;
    server_line(
        &mut server,
        b":ergo.test REGISTER VERIFICATION_REQUIRED alice :Verification code sent\r\n",
    )
    .await;

    assert!(within(waiter).await.unwrap().is_ok());
    assert!(matches!(
        events.next().await,
        Some(Event::AccountCreation(crate::AccountCreation::VerificationRequired { account, message }))
            if account == "alice" && message == "Verification code sent"
    ));
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn registration_sets_advertised_bot_mode_and_joins_each_channel() {
    let (client_stream, mut server) = tokio::io::duplex(4096);
    let (_client, mut events, driver) = Driver::with_stream_and_connect(
        client_stream,
        Session::new(config()),
        super::DEFAULT_CHANNEL_CAPACITY,
        None,
        super::PostConnect {
            bot: true,
            channels: vec![
                "#papo".into(),
                super::Autojoin::new("#PAPO", Some("replacement-channel-key".into())),
                super::Autojoin::new("#papo-bots", Some("private-channel-key".into())),
            ],
            parted_while_disconnected: Vec::new(),
            history_on_reconnect: false,
            disconnected_at: None,
        },
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
    client_line(&mut server, b"CAP END\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver BOT=B :are supported by this server\r\n:ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Connected(_))));
    client_line(
        &mut server,
        b"MODE papo-driver +B\r\nJOIN #PAPO replacement-channel-key\r\nJOIN #papo-bots private-channel-key\r\n",
    )
    .await;
    server_line(
        &mut server,
        b":ergo.test 473 papo-driver #PAPO :Cannot join channel (+i)\r\n",
    )
    .await;
    assert!(matches!(
        events.next().await,
        Some(Event::AutoJoinFailed { channel, error })
            if channel == "#PAPO" && error.refusal() == crate::Refusal::InviteOnlyChannel
    ));
    server_line(
        &mut server,
        b":ergo.test NOTICE papo-driver :Autojoin refusal recorded\r\n",
    )
    .await;
    assert_eq!(
        next_message(&mut events).await.text,
        "Autojoin refusal recorded"
    );
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn bot_default_without_advertised_mode_sends_no_mode() {
    assert_connect_output(
        super::PostConnect {
            bot: true,
            channels: Vec::new(),
            parted_while_disconnected: Vec::new(),
            history_on_reconnect: false,
            disconnected_at: None,
        },
        None,
        b"",
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn advertised_bot_mode_without_bot_default_sends_no_mode() {
    assert_connect_output(
        super::PostConnect::default(),
        Some(b":ergo.test 005 papo-driver BOT=B :are supported by this server\r\n"),
        b"",
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn empty_channel_defaults_send_no_join() {
    assert_connect_output(
        super::PostConnect {
            bot: true,
            channels: Vec::new(),
            parted_while_disconnected: Vec::new(),
            history_on_reconnect: false,
            disconnected_at: None,
        },
        Some(b":ergo.test 005 papo-driver BOT=B :are supported by this server\r\n"),
        b"MODE papo-driver +B\r\n",
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn cap_new_and_del_forward_capability_changes() {
    let (mut server, _client, mut events, driver) = scripted(config().wanted_caps(["server-time"]));
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
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
        matches!(events.next().await, Some(Event::CapsChanged(changed))
        if changed.added.get("server-time") == Some(&Some("2027".to_owned())))
    );
    server_line(
        &mut server,
        b":ergo.test CAP papo-driver DEL :server-time\r\n",
    )
    .await;
    assert!(
        matches!(events.next().await, Some(Event::CapsChanged(changed))
        if changed.removed.contains_key("server-time"))
    );
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn plaintext_sts_forwards_upgrade_and_closes() {
    let settings = config().host("irc.example.net").wanted_caps(["sts"]);
    let (mut server, _client, mut events, driver) = scripted(settings);
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :sts=port=6697\r\n").await;
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Protocol { message },
            retry_in: None,
        })) if message == "sts upgrade"
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
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
    let (registration, waiter) = tokio::sync::oneshot::channel();
    let (client, mut events, driver) = Driver::with_connector(
        Ok(ConnectionPlan {
            address: "irc://irc.example.net".parse().unwrap(),
            tls: TlsConfig::default(),
        }),
        Session::new(config().host("irc.example.net").wanted_caps(["sts"])),
        super::DEFAULT_CHANNEL_CAPACITY,
        Some(registration),
        super::PostConnect::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut first_server).await;
    server_line(&mut first_server, b":ergo.test CAP * LS :sts=port=6699\r\n").await;
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            retry_in: Some(Duration::ZERO),
            ..
        }))
    ));
    welcome_burst(&mut second_server).await;
    assert_eq!(&*requested.lock().unwrap(), &[(false, 6667), (true, 6699)]);
    server_line(
        &mut second_server,
        b":ergo.test CAP * LS :sts=duration=3600\r\n",
    )
    .await;
    client_line(&mut second_server, b"CAP END\r\n").await;
    server_line(
        &mut second_server,
        b":ergo.test 001 papo-driver :Welcome over TLS\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
    )
    .await;
    assert!(within(waiter).await.unwrap().is_ok());

    let quit = tokio::spawn(async move { client.quit("").await });
    client_line(&mut second_server, b"QUIT\r\n").await;
    within(second_server.shutdown()).await.unwrap();
    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
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
        super::PostConnect::default(),
        Reconnect::exponential().max_attempts(1),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :sts=port=6697\r\n").await;
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if !disconnected.is_retrying()
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[test]
fn consecutive_sts_upgrades_are_a_typed_error() {
    let mut last_host = None;
    assert!(super::record_sts_upgrade(&mut last_host, "irc.example.net").is_ok());
    assert!(matches!(
        super::record_sts_upgrade(&mut last_host, "irc.example.net"),
        Err(Error::Connect(error))
            if error.kind() == crate::ConnectErrorKind::StsUpgradeLoop
                && std::error::Error::source(&error)
                    .is_some_and(|source| source.to_string().contains("irc.example.net"))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn secure_sts_forwards_persistence_policy() {
    let settings = config()
        .connection_security(ConnectionSecurity::Secure)
        .host("IRC.Example.NET.")
        .wanted_caps(["sts"]);
    let (mut server, _client, _events, driver) = scripted(settings);
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(
        &mut server,
        b":ergo.test CAP * LS :sts=duration=3600,preload\r\n",
    )
    .await;
    client_line(&mut server, b"CAP END\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
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
    let (_client, _events, driver) = Driver::with_connector(
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
        super::PostConnect::default(),
        Reconnect::never(),
        Box::new(store),
        connector,
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :sts=duration=3600\r\n").await;
    within(async {
        while papo_core::sts::StsStore::get(&inspection, "irc.example.net").is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let mut policy = papo_core::sts::StsStore::get(&inspection, "irc.example.net").unwrap();
    assert_eq!(policy.port, 7443);
    assert_eq!(policy.duration, Duration::from_secs(3600));
    assert!(policy.expires_at > std::time::SystemTime::now());
    policy.expires_at = SystemTime::UNIX_EPOCH;
    papo_core::sts::StsStore::put(&mut inspection, policy);
    client_line(&mut server, b"CAP END\r\n").await;
    let disconnect_started = SystemTime::now();
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
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
        papo_core::sts::StsPolicy::new(
            "irc.example.net",
            6697,
            SystemTime::now() + Duration::from_secs(3600),
            Duration::from_secs(3600),
        ),
    );
    let inspection = store.clone();
    let (_client, _events, driver) = Driver::with_connector(
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
        super::PostConnect::default(),
        Reconnect::never(),
        Box::new(store),
        connector,
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :sts=duration=0\r\n").await;
    within(async {
        while papo_core::sts::StsStore::get(&inspection, "irc.example.net").is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(papo_core::sts::StsStore::get(&inspection, "irc.example.net").is_none());
    client_line(&mut server, b"CAP END\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
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
        super::PostConnect::default(),
        Reconnect::never(),
        Box::new(store),
        connector,
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
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
    client_line(&mut server, b"CAP END\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn client_send_raw_obeys_the_rate_limit() {
    let settings = config()
        .ping_interval(Duration::from_secs(200))
        .ping_timeout(Duration::from_secs(200))
        .rate_limit(
            papo_core::RateLimit::new(5, Duration::from_millis(500))
                .unwrap()
                .strict(),
        );
    let (mut server, client, mut events, driver) = scripted(settings);
    let task = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    let started = crate::rt::now_mono();
    for index in 0..200 {
        let nick = format!("release-bot-{index}");
        client
            .send_raw(crate::Line::new("WHOIS", [&nick]))
            .await
            .unwrap();
        client_line(&mut server, format!("WHOIS {nick}\r\n").as_bytes()).await;
    }
    assert_eq!(
        crate::rt::now_mono().duration_since(started),
        Duration::from_millis(97_500)
    );
    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn queued_raw_sends_fail_when_the_connection_drops() {
    let (mut server, client, _events, driver) = scripted(
        config().rate_limit(
            papo_core::RateLimit::new(1, Duration::from_hours(24))
                .unwrap()
                .strict(),
        ),
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    client
        .send_raw(crate::Line::new("WHOIS", ["release-bot"]))
        .await
        .unwrap();
    client_line(&mut server, b"WHOIS release-bot\r\n").await;

    let queued: Vec<_> = (0..3)
        .map(|index| {
            let client = client.clone();
            tokio::spawn(async move {
                client
                    .send_raw(crate::Line::new(
                        "WHOIS",
                        [format!("queued-release-bot-{index}")],
                    ))
                    .await
            })
        })
        .collect();
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    within(server.shutdown()).await.unwrap();
    for result in queued {
        assert!(matches!(
            within(result).await.unwrap(),
            Err(disconnected!())
        ));
    }
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn queued_raw_send_preserves_the_server_disconnect_cause() {
    let (mut server, client, _events, driver) = scripted(
        config().rate_limit(
            papo_core::RateLimit::new(1, Duration::from_hours(24))
                .unwrap()
                .strict(),
        ),
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    client
        .send_raw(crate::Line::new("WHOIS", ["release-bot"]))
        .await
        .unwrap();
    client_line(&mut server, b"WHOIS release-bot\r\n").await;

    let queued = tokio::spawn(async move {
        client
            .send_raw(crate::Line::new("WHOIS", ["queued-release-bot"]))
            .await
    });
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    server_line(
        &mut server,
        b"ERROR :Closing Link: papo-driver (Server maintenance)\r\n",
    )
    .await;

    assert!(matches!(
        within(queued).await.unwrap(),
        Err(Error::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Server { message },
            retry_in: None,
        })) if message == "Closing Link: papo-driver (Server maintenance)"
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn raw_line_builder_quotes_real_commands() {
    let (mut server, client, _events, driver) = scripted(
        config().rate_limit(papo_core::RateLimit::new(1, Duration::from_millis(500)).unwrap()),
    );
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;

    client
        .send_raw(crate::Line::new("KILL", ["alice", "policy violation"]))
        .await
        .unwrap();
    client
        .send_raw(crate::Line::new("WALLOPS", ["maintenance now"]))
        .await
        .unwrap();
    client_line(&mut server, b"KILL alice :policy violation\r\n").await;
    client_line(&mut server, b"WALLOPS :maintenance now\r\n").await;

    client
        .send_raw(crate::Line::new("WHOWAS", ["emily", "2"]))
        .await
        .unwrap();
    client_line(&mut server, b"WHOWAS emily 2\r\n").await;
    client
        .send_raw(crate::Line::new("MOTD", Vec::<String>::new()))
        .await
        .unwrap();
    client_line(&mut server, b"MOTD\r\n").await;

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn join_rejects_an_invalid_channel_name_before_queueing() {
    let client = crate::Client::disconnected_for_test();
    assert!(matches!(
        client.join(test_channel("rust")).await,
        Err(Error::InvalidInput(crate::InvalidInput::InvalidName {
            name,
            reason: crate::InvalidIdentifier::InvalidPrefix('r'),
        })) if name == "rust"
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn raw_request_returns_every_line_from_a_labelled_batch() {
    let (mut server, client, mut events, driver) =
        scripted(config().wanted_caps(["labeled-response"]));
    let driver = tokio::spawn(driver);
    complete_connect_labelled(&mut server, &mut events).await;

    let request = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request_raw(crate::Line::new("MOTD", Vec::<String>::new()))
                .await
        }
    });
    client_line(&mut server, b"@label=0 MOTD\r\n").await;
    server_line(&mut server, b"@label=0 :ergo.test BATCH +motd labeled-response\r\n@batch=motd :ergo.test 375 papo-driver :- Ergo Message of the Day -\r\n@batch=motd :ergo.test 372 papo-driver :- Welcome to ErgoTest\r\n@batch=motd :ergo.test 376 papo-driver :End of /MOTD command.\r\n:ergo.test BATCH -motd\r\n").await;
    let lines = within(request).await.unwrap().unwrap();
    assert_eq!(
        lines.iter().map(crate::Line::command).collect::<Vec<_>>(),
        ["375", "372", "376"]
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn raw_request_returns_no_lines_for_an_ack() {
    let (mut server, client, mut events, driver) =
        scripted(config().wanted_caps(["labeled-response"]));
    let driver = tokio::spawn(driver);
    complete_connect_labelled(&mut server, &mut events).await;

    let request = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request_raw(crate::Line::new("MODE", ["#papo", "+i"]))
                .await
        }
    });
    client_line(&mut server, b"@label=0 MODE #papo +i\r\n").await;
    server_line(&mut server, b"@label=0 :ergo.test ACK\r\n").await;
    assert!(within(request).await.unwrap().unwrap().is_empty());

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

fn assert_structured_who(who: &[crate::WhoEntry]) {
    assert_eq!(who.len(), 2);
    assert_eq!(
        (&who[0].nick, &who[0].account),
        (
            &test_nick("alice"),
            &crate::Account::LoggedIn(test_account("alice"))
        )
    );
    assert_eq!(
        (&who[0].away, who[0].is_bot, who[0].prefixes.as_slice()),
        (&crate::Away::Present, true, ['@'].as_slice())
    );
    assert_eq!(
        (
            &who[1].account,
            &who[1].away,
            who[1].is_oper,
            who[1].prefixes.as_slice()
        ),
        (
            &crate::Account::LoggedOut,
            &crate::Away::Away { message: None },
            true,
            ['+'].as_slice()
        )
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn structured_requests_resolve_fail_and_abort() {
    let (mut server, client, mut events, driver) = scripted(
        config()
            .wanted_caps(["labeled-response"])
            .request_timeout(Duration::from_secs(1)),
    );
    let driver = tokio::spawn(driver);
    complete_connect_labelled(&mut server, &mut events).await;

    let whois = tokio::spawn({
        let client = client.clone();
        async move { client.whois(test_nick("alice")).await }
    });
    client_line(&mut server, b"@label=0 WHOIS alice\r\n").await;
    server_line(&mut server, b"@label=0 :ergo.test BATCH +whois-42 labeled-response\r\n@batch=whois-42 :ergo.test 311 papo-driver alice ~alice client.example * :Alice Example\r\n@batch=whois-42 :ergo.test 312 papo-driver alice ergo.test :Ergo Test Network\r\n@batch=whois-42 :ergo.test 317 papo-driver alice 42 1760544000 :seconds idle, signon time\r\n@batch=whois-42 :ergo.test 319 papo-driver alice :@#papo +#rust\r\n@batch=whois-42 :ergo.test 330 papo-driver alice alice :is logged in as\r\n@batch=whois-42 :ergo.test 301 papo-driver alice :away from keyboard\r\n@batch=whois-42 :ergo.test 335 papo-driver alice :is a bot\r\n@batch=whois-42 :ergo.test 671 papo-driver alice :is using a secure connection\r\n@batch=whois-42 :ergo.test 318 papo-driver alice :End of /WHOIS list\r\n:ergo.test BATCH -whois-42\r\n").await;
    let whois = within(whois).await.unwrap().unwrap();
    assert_eq!(
        (whois.nick.as_ref(), whois.username.as_deref()),
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
    assert_eq!(whois.channels[0].name, "#papo");
    assert_eq!(whois.channels[0].prefixes, ['@']);
    assert_eq!(whois.channels[1].name, "#rust");
    assert_eq!(whois.channels[1].prefixes, ['+']);
    assert_eq!(
        whois.away,
        crate::Away::Away {
            message: Some("away from keyboard".into())
        }
    );
    assert!(whois.is_bot && whois.secure);
    assert_eq!(whois.idle, Some(Duration::from_secs(42)));

    server_line(
        &mut server,
        b":ergo.test 005 papo-driver WHOX PREFIX=(ov)@+ CHANMODES=beI,k,l,imnst :are supported\r\n",
    )
    .await;
    let who = tokio::spawn({
        let client = client.clone();
        async move { client.who(test_mask("#papo")).await }
    });
    client_line(&mut server, b"@label=1 WHO #papo %tcuhnfar,152\r\n").await;
    server_line(&mut server, b"@label=1 :ergo.test BATCH +who-42 labeled-response\r\n@batch=who-42 :ergo.test 354 papo-driver 152 #papo ~alice client.example alice H@B alice :Alice Example\r\n@batch=who-42 :ergo.test 354 papo-driver 152 #papo ~dan user.example dan G+* 0 :Dan Away\r\n@batch=who-42 :ergo.test 315 papo-driver #papo :End of WHO list\r\n:ergo.test BATCH -who-42\r\n").await;
    let who = within(who).await.unwrap().unwrap();
    assert_structured_who(&who);

    assert_join_waits_for_sync(&mut server, &client).await;
    assert_channel_requests(&mut server, &client).await;

    let request = tokio::spawn({
        let client = client.clone();
        async move { client.whois(test_nick("ghost")).await }
    });
    client_line(&mut server, b"@label=7 WHOIS ghost\r\n").await;
    server_line(
        &mut server,
        b"@label=7 :ergo.test 401 papo-driver ghost :No such nick\r\n",
    )
    .await;
    assert!(matches!(
        within(request).await.unwrap(),
        Err(Error::Server(error)) if error.code() == "401"
    ));
    let aborted = tokio::spawn(async move {
        client
            .request_raw(crate::Line::new("WHOIS", ["emily"]))
            .await
    });
    client_line(&mut server, b"@label=8 WHOIS emily\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(aborted).await.unwrap(),
        Err(Error::Interrupted(crate::Disconnected {
            cause: crate::DisconnectCause::ConnectionLost { .. },
            retry_in: None,
        }))
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn raw_request_times_out_after_it_is_written() {
    let (mut server, client, mut events, driver) = scripted(
        config()
            .wanted_caps(["labeled-response"])
            .request_timeout(Duration::from_secs(1)),
    );
    let driver = tokio::spawn(driver);
    complete_connect_labelled(&mut server, &mut events).await;

    let timed_out = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request_raw(crate::Line::new("WHOIS", ["alice"]))
                .await
        }
    });
    client_line(&mut server, b"@label=0 WHOIS alice\r\n").await;
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        within(timed_out).await.unwrap(),
        Err(Error::Timeout)
    ));
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn raw_request_returns_state_changing_lines_without_an_event() {
    let (mut server, client, mut events, driver) =
        scripted(config().wanted_caps(["labeled-response"]));
    let driver = tokio::spawn(driver);
    complete_connect_labelled(&mut server, &mut events).await;

    let renamed = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request_raw(crate::Line::new("NICK", ["papo-renamed"]))
                .await
        }
    });
    client_line(&mut server, b"@label=0 NICK papo-renamed\r\n").await;
    server_line(
        &mut server,
        b"@label=0 :papo-driver!papo@localhost NICK :papo-renamed\r\n",
    )
    .await;
    let lines = within(renamed).await.unwrap().unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].command(), "NICK");
    assert_eq!(lines[0].params(), ["papo-renamed"]);
    assert_eq!(client.nick(), "papo-renamed");
    assert!(
        tokio::time::timeout(Duration::from_millis(1), events.next())
            .await
            .is_err()
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn identity_and_away_actions_match_unlabelled_confirmations() {
    let (mut server, client, mut events, driver) = scripted(config().wanted_caps(["setname"]));
    let driver = tokio::spawn(driver);
    complete_connect_caps(&mut server, &mut events, "setname", "setname").await;

    let rename = tokio::spawn({
        let client = client.clone();
        async move { client.set_nick(test_nick("paporev")).await }
    });
    client_line(&mut server, b"NICK paporev\r\n").await;
    server_line(
        &mut server,
        b":papo-driver!papo@localhost NICK :paporev\r\n",
    )
    .await;
    within(rename).await.unwrap().unwrap();
    assert_eq!(client.state().nick(), "paporev");

    let away = tokio::spawn({
        let client = client.clone();
        async move { client.set_away("reviewing patches").await }
    });
    client_line(&mut server, b"AWAY :reviewing patches\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 306 paporev :You have been marked as being away\r\n",
    )
    .await;
    within(away).await.unwrap().unwrap();

    let back = tokio::spawn({
        let client = client.clone();
        async move { client.clear_away().await }
    });
    client_line(&mut server, b"AWAY\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 305 paporev :You are no longer marked as being away\r\n",
    )
    .await;
    within(back).await.unwrap().unwrap();

    let realname = tokio::spawn({
        let client = client.clone();
        async move { client.set_realname("Papo Review Client").await }
    });
    client_line(&mut server, b"SETNAME :Papo Review Client\r\n").await;
    server_line(
        &mut server,
        b":paporev!papo@localhost SETNAME :Papo Review Client\r\n",
    )
    .await;
    within(realname).await.unwrap().unwrap();

    assert!(matches!(
        client.set_away("").await,
        Err(Error::InvalidInput(crate::InvalidInput::EmptyText))
    ));
    assert!(matches!(
        client.set_realname("").await,
        Err(Error::InvalidInput(crate::InvalidInput::EmptyText))
    ));
    assert!(matches!(
        client.set_nick("").await,
        Err(Error::InvalidInput(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn own_join_after_a_nick_change_is_marked_as_self() {
    let (mut server, client, mut events, driver) =
        scripted(config().rate_limit(papo_core::RateLimit::unlimited()));
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    client
        .send_raw(crate::Line::new("NICK", ["papo-renamed"]))
        .await
        .unwrap();
    client_line(&mut server, b"NICK papo-renamed\r\n").await;
    server_line(
        &mut server,
        b":PAPO-DRIVER!papo@localhost NICK :papo-renamed\r\n",
    )
    .await;
    let changed = events
        .next()
        .await
        .unwrap()
        .as_nick_changed()
        .unwrap()
        .clone();
    assert_eq!(changed.new, "papo-renamed");
    assert_eq!(changed.authorship, crate::Authorship::Own);

    server_line(
        &mut server,
        b":papo-renamed!papo@localhost JOIN #papo-self * :Papo Client\r\n",
    )
    .await;
    let joined = events.next().await.unwrap().as_joined().unwrap().clone();
    assert_eq!(joined.actor.nick().map(AsRef::as_ref), Some("papo-renamed"));
    assert_eq!(joined.nick, "papo-renamed");
    assert_eq!(joined.channel, "#papo-self");
    assert_eq!(joined.authorship, crate::Authorship::Own);
    server_line(
        &mut server,
        b":ergo.test 353 papo-renamed = #papo-self :@papo-renamed\r\n:ergo.test 366 papo-renamed #papo-self :End of /NAMES list\r\n",
    )
    .await;
    client_line(&mut server, b"MODE #papo-self\r\n").await;
    client_line(&mut server, b"WHO #papo-self\r\n").await;

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn membership_actions_confirm_success_and_refusal() {
    let (mut server, client, mut events, driver) =
        scripted(config().wanted_caps(["labeled-response"]));
    let driver = tokio::spawn(driver);
    complete_connect_labelled(&mut server, &mut events).await;

    let invite = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .invite(test_channel("#papo"), test_nick("carol"))
                .await
        }
    });
    client_line(&mut server, b"@label=0 INVITE carol #papo\r\n").await;
    server_line(
        &mut server,
        b"@label=0 :ergo.test 341 papo-driver carol #papo\r\n",
    )
    .await;
    within(invite).await.unwrap().unwrap();

    let kick = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .kick(test_channel("#papo"), test_nick("spambot"), "")
                .await
        }
    });
    client_line(&mut server, b"@label=1 KICK #papo spambot\r\n").await;
    server_line(
        &mut server,
        b"@label=1 :ergo.test 482 papo-driver #papo :You're not a channel operator\r\n",
    )
    .await;
    assert!(matches!(
        within(kick).await.unwrap(),
        Err(Error::Server(error)) if error.code() == "482"
    ));

    assert!(matches!(
        client
            .invite(test_channel("not-a-channel"), test_nick("bad,nick"))
            .await,
        Err(Error::InvalidInput(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn kick_reasons_obey_advertised_byte_limits() {
    let (mut server, client, mut events, driver) =
        scripted(config().rate_limit(papo_core::RateLimit::unlimited()));
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;

    for maximum in [390, 307, 300] {
        server_line(
            &mut server,
            format!(":ergo.test 005 papo-driver KICKLEN={maximum} :are supported\r\n").as_bytes(),
        )
        .await;
        within(async {
            while client.state().isupport().kicklen() != Some(maximum) {
                tokio::task::yield_now().await;
            }
        })
        .await;

        let exact = format!("{}{}", "é".repeat(maximum / 2), "x".repeat(maximum % 2));
        let kick = tokio::spawn({
            let client = client.clone();
            let exact = exact.clone();
            async move { client.kick("#papo", "spambot", exact).await }
        });
        client_line(
            &mut server,
            format!("KICK #papo spambot {exact}\r\n").as_bytes(),
        )
        .await;
        server_line(
            &mut server,
            format!(":papo-driver!papo@localhost KICK #papo spambot :{exact}\r\n").as_bytes(),
        )
        .await;
        within(kick).await.unwrap().unwrap();

        let too_long = format!("{exact}é");
        assert!(matches!(
            client.kick("#papo", "spambot", too_long).await,
            Err(Error::InvalidInput(crate::InvalidInput::KickReasonTooLong {
                maximum: advertised,
                actual,
            })) if advertised == maximum && actual > maximum
        ));
        assert!(matches!(
            client
                .kickban("#papo", "spambot", format!("{exact}é"), "spambot")
                .await,
            Err(Error::InvalidInput(crate::InvalidInput::KickReasonTooLong {
                maximum: advertised,
                actual,
            })) if advertised == maximum && actual > maximum
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(1), server.read_u8())
                .await
                .is_err()
        );
    }

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn kickban_confirms_the_ban_before_returning_the_kick_result() {
    let (mut server, client, mut events, driver) =
        scripted(config().wanted_caps(["labeled-response"]));
    let driver = tokio::spawn(driver);
    complete_connect_labelled(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver CHANMODES=beI,k,l,imnst :are supported\r\n",
    )
    .await;

    let success = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .kickban("#papo", "alice", "repeated flooding", "alice")
                .await
        }
    });
    client_line(&mut server, b"@label=0 MODE #papo +b alice\r\n").await;
    server_line(
        &mut server,
        b"@label=0 :papo-driver!papo@localhost MODE #papo +b alice!*@*\r\n",
    )
    .await;
    client_line(
        &mut server,
        b"@label=1 KICK #papo alice :repeated flooding\r\n",
    )
    .await;
    server_line(
        &mut server,
        b"@label=1 :papo-driver!papo@localhost KICK #papo alice :repeated flooding\r\n",
    )
    .await;
    assert!(within(success).await.unwrap().unwrap().kick.is_ok());

    let refused = tokio::spawn({
        let client = client.clone();
        async move { client.kickban("#papo", "carol", "abuse", "carol!*@*").await }
    });
    client_line(&mut server, b"@label=2 MODE #papo +b carol!*@*\r\n").await;
    server_line(
        &mut server,
        b"@label=2 :ergo.test 482 papo-driver #papo :You're not a channel operator\r\n",
    )
    .await;
    assert!(matches!(
        within(refused).await.unwrap(),
        Err(Error::Server(error))
            if error.refusal() == crate::Refusal::ChannelOperatorPrivilegesNeeded
    ));

    let duplicate = tokio::spawn({
        let client = client.clone();
        async move { client.kickban("#papo", "bob", "abuse", "bob!*@*").await }
    });
    client_line(&mut server, b"@label=3 MODE #papo +b bob!*@*\r\n").await;
    server_line(
        &mut server,
        b"@label=3 :irc.inspircd.test 697 papo-driver #papo bob!*@* b :Entry is already in the list\r\n",
    )
    .await;
    client_line(&mut server, b"@label=4 KICK #papo bob abuse\r\n").await;
    server_line(
        &mut server,
        b"@label=4 :irc.inspircd.test 484 papo-driver bob #papo :Cannot kick protected user\r\n",
    )
    .await;
    let outcome = within(duplicate).await.unwrap().unwrap();
    assert!(matches!(
        outcome.kick,
        Err(Error::Server(error)) if error.refusal() == crate::Refusal::CannotKickProtected
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn kickban_uses_a_list_fallback_and_skips_an_already_tracked_ban() {
    let (mut server, client, mut events, driver) =
        scripted(config().wanted_caps(["labeled-response"]));
    let driver = tokio::spawn(driver);
    complete_connect_labelled(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver CHANMODES=beI,k,l,imnst :are supported\r\n",
    )
    .await;

    let fetched = tokio::spawn({
        let client = client.clone();
        async move { client.kickban("#papo", "alice", "flooding", "alice").await }
    });
    client_line(&mut server, b"@label=0 MODE #papo +b alice\r\n").await;
    server_line(&mut server, b"@label=0 ACK\r\n").await;
    client_line(&mut server, b"@label=1 MODE #papo b\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 367 papo-driver #papo alice!*@* oper 1792573200\r\n:ergo.test 368 papo-driver #papo :End of channel ban list\r\n",
    )
    .await;
    client_line(&mut server, b"@label=2 KICK #papo alice flooding\r\n").await;
    server_line(
        &mut server,
        b"@label=2 :papo-driver!papo@localhost KICK #papo alice :flooding\r\n",
    )
    .await;
    assert!(within(fetched).await.unwrap().unwrap().kick.is_ok());

    server_line(
        &mut server,
        b":papo-driver!papo@localhost JOIN #papo\r\n:ergo.test 353 papo-driver = #papo :@papo-driver bob\r\n",
    )
    .await;
    let list = tokio::spawn({
        let client = client.clone();
        async move { client.list_modes("#papo", crate::ListMode::Ban).await }
    });
    client_line(&mut server, b"@label=3 MODE #papo b\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 367 papo-driver #papo bob!*@* oper 1792573200\r\n:ergo.test 368 papo-driver #papo :End of channel ban list\r\n",
    )
    .await;
    assert_eq!(within(list).await.unwrap().unwrap().len(), 1);

    let tracked = tokio::spawn({
        let client = client.clone();
        async move { client.kickban("#papo", "bob", "flooding", "bob").await }
    });
    client_line(&mut server, b"@label=4 KICK #papo bob flooding\r\n").await;
    server_line(
        &mut server,
        b"@label=4 :papo-driver!papo@localhost KICK #papo bob :flooding\r\n",
    )
    .await;
    assert!(within(tracked).await.unwrap().unwrap().kick.is_ok());

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn kickban_stops_when_the_ban_is_unconfirmed_or_the_target_leaves() {
    let (mut server, client, mut events, driver) =
        scripted(config().wanted_caps(["labeled-response"]));
    let driver = tokio::spawn(driver);
    complete_connect_labelled(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver CHANMODES=beI,k,l,imnst :are supported\r\n",
    )
    .await;

    let unconfirmed = tokio::spawn({
        let client = client.clone();
        async move { client.kickban("#papo", "alice", "flooding", "alice").await }
    });
    client_line(&mut server, b"@label=0 MODE #papo +b alice\r\n").await;
    server_line(&mut server, b"@label=0 ACK\r\n").await;
    client_line(&mut server, b"@label=1 MODE #papo b\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 368 papo-driver #papo :End of channel ban list\r\n",
    )
    .await;
    assert!(matches!(
        within(unconfirmed).await.unwrap(),
        Err(Error::Unconfirmed)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );

    server_line(
        &mut server,
        b":papo-driver!papo@localhost JOIN #papo\r\n:ergo.test 353 papo-driver = #papo :@papo-driver carol\r\n",
    )
    .await;
    within(async {
        while client
            .state()
            .channel("#papo")
            .is_none_or(|channel| channel.member("carol").is_none())
        {
            tokio::task::yield_now().await;
        }
    })
    .await;

    let departed = tokio::spawn({
        let client = client.clone();
        async move { client.kickban("#papo", "carol", "flooding", "carol").await }
    });
    client_line(&mut server, b"@label=2 MODE #papo +b carol\r\n").await;
    server_line(
        &mut server,
        b":carol!~carol@users.example NICK :carol-away\r\n@label=2 :papo-driver!papo@localhost MODE #papo +b carol!*@*\r\n",
    )
    .await;
    let outcome = within(departed).await.unwrap().unwrap();
    assert!(matches!(outcome.kick, Err(Error::TargetGone)));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn typed_numeric_refusals_reach_action_futures() {
    let (mut server, client, mut events, driver) =
        scripted(config().rate_limit(papo_core::RateLimit::unlimited()));
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;

    for (channel, numeric, refusal, source, description) in [
        (
            "#invite",
            473,
            crate::Refusal::InviteOnlyChannel,
            "ergo.test",
            "Cannot join channel (+i)",
        ),
        (
            "#banned",
            474,
            crate::Refusal::BannedFromChannel,
            "irc.solanum.test",
            "Cannot join channel (+b)",
        ),
        (
            "#keyed",
            475,
            crate::Refusal::BadChannelKey,
            "ergo.test",
            "Cannot join channel (+k)",
        ),
    ] {
        assert_action_refusal(
            &mut server,
            client.join(channel).into_future(),
            &format!("JOIN {channel}\r\n"),
            &format!(":{source} {numeric} papo-driver {channel} :{description}\r\n"),
            numeric,
            refusal,
        )
        .await;
    }

    for (channel, numeric, refusal, incoming) in [
        (
            "#papo",
            441,
            crate::Refusal::UserNotInChannel,
            ":irc.solanum.test 441 papo-driver carol #papo :They aren't on that channel\r\n",
        ),
        (
            "#staff",
            482,
            crate::Refusal::ChannelOperatorPrivilegesNeeded,
            ":ergo.test 482 papo-driver #staff :You're not a channel operator\r\n",
        ),
    ] {
        assert_action_refusal(
            &mut server,
            client.kick(channel, "carol", ""),
            &format!("KICK {channel} carol\r\n"),
            incoming,
            numeric,
            refusal,
        )
        .await;
    }

    for (nick, numeric, refusal, description) in [
        (
            "42papo",
            432,
            crate::Refusal::ErroneousNickname,
            "Erroneous nickname",
        ),
        (
            "alice",
            433,
            crate::Refusal::NicknameInUse,
            "Nickname is already in use",
        ),
    ] {
        assert_action_refusal(
            &mut server,
            client.set_nick(nick),
            &format!("NICK {nick}\r\n"),
            &format!(":ergo.test {numeric} papo-driver {nick} :{description}\r\n"),
            numeric,
            refusal,
        )
        .await;
    }

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn typed_mode_refusals_reach_action_futures() {
    let (mut server, client, mut events, driver) =
        scripted(config().rate_limit(papo_core::RateLimit::unlimited()));
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver CHANMODES=beI,k,l,imnstz :are supported\r\n",
    )
    .await;
    within(async {
        while !matches!(
            client.state().isupport().get("CHANMODES"),
            crate::isupport::Advertised::Value("beI,k,l,imnstz")
        ) {
            tokio::task::yield_now().await;
        }
    })
    .await;

    for (id, target, mode, numeric, refusal, incoming) in [
        (
            0,
            "#papo",
            'z',
            472,
            crate::Refusal::UnknownMode,
            ":irc.solanum.test 472 papo-driver z :is unknown mode char to me",
        ),
        (
            1,
            "#staff",
            'm',
            482,
            crate::Refusal::ChannelOperatorPrivilegesNeeded,
            ":ergo.test 482 papo-driver #staff :You're not a channel operator",
        ),
    ] {
        assert_action_refusal(
            &mut server,
            client.mode(target).add(mode).into_future(),
            &format!("MODE {target} +{mode}\r\nPING :papo-mode-{id}\r\n"),
            &format!("{incoming}\r\n:ergo.test PONG ergo.test :papo-mode-{id}\r\n"),
            numeric,
            refusal,
        )
        .await;
    }

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn mode_helpers_await_every_command_and_return_refusals() {
    let (mut server, client, mut events, driver) =
        scripted(config().wanted_caps(["labeled-response"]));
    let driver = tokio::spawn(driver);
    complete_connect_labelled(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver PREFIX=(ov)@+ CHANMODES=beI,k,l,imnst MODES=2 :are supported\r\n",
    )
    .await;

    let modes = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .mode("#papo")
                .op("alice")
                .voice("carol")
                .ban("*!*@spam.example")
                .await
        }
    });
    client_line(&mut server, b"@label=0 MODE #papo +ov alice carol\r\n").await;
    server_line(
        &mut server,
        b"@label=0 :papo-driver!papo@localhost MODE #papo +ov alice carol\r\n",
    )
    .await;
    client_line(&mut server, b"@label=1 MODE #papo +b *!*@spam.example\r\n").await;
    assert!(!modes.is_finished());
    server_line(
        &mut server,
        b"@label=1 :papo-driver!papo@localhost MODE #papo +b *!*@spam.example\r\n",
    )
    .await;
    let result = within(modes).await.unwrap().unwrap();
    assert_eq!(result.applied.len(), 3);
    assert!(
        matches!(result.applied[0].arg, Some(crate::ModeArg::Nick(ref nick)) if nick == "alice")
    );
    assert!(
        matches!(result.applied[2].arg, Some(crate::ModeArg::Mask(ref mask)) if mask == "*!*@spam.example")
    );

    let refused = tokio::spawn({
        let client = client.clone();
        async move { client.mode("#papo").deop(test_nick("alice")).await }
    });
    client_line(&mut server, b"@label=2 MODE #papo -o alice\r\n").await;
    server_line(
        &mut server,
        b"@label=2 :ergo.test 482 papo-driver #papo :You're not a channel operator\r\n",
    )
    .await;
    assert!(matches!(
        within(refused).await.unwrap(),
        Err(Error::Server(error)) if error.code() == "482"
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn mode_changes_report_an_applied_change_and_a_partial_refusal() {
    let (mut server, client, mut events, driver) =
        scripted(config().rate_limit(papo_core::RateLimit::unlimited()));
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver CHANMODES=beI,k,l,imnst MODES=2 :are supported\r\n",
    )
    .await;

    let modes = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .mode("#papo")
                .ban("alice!*@*")
                .ban("spambot!*@gateway.example")
                .await
        }
    });
    client_line(
        &mut server,
        b"MODE #papo +bb alice!*@* spambot!*@gateway.example\r\nPING :papo-mode-0\r\n",
    )
    .await;
    server_line(
        &mut server,
        b":papo-driver!~papo@localhost MODE #papo +b alice!*@*\r\n:ergo.test 478 papo-driver #papo b :Channel ban list is full\r\n:ergo.test PONG ergo.test :papo-mode-0\r\n",
    )
    .await;

    let result = within(modes).await.unwrap().unwrap();
    assert!(
        matches!(result.applied.as_slice(), [change] if matches!(change.arg, Some(crate::ModeArg::Mask(ref mask)) if mask == "alice!*@*"))
    );
    assert!(
        matches!(result.refused.as_slice(), [(change, crate::Refusal::BanListFull)] if matches!(change.arg, Some(crate::ModeArg::Mask(ref mask)) if mask == "spambot!*@gateway.example"))
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn mode_changes_validate_every_chunk_before_sending() {
    let (mut server, client, mut events, driver) =
        scripted(config().rate_limit(papo_core::RateLimit::unlimited()));
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver CHANMODES=beI,k,l,imnst MODES=1 :are supported\r\n",
    )
    .await;

    assert!(matches!(
        client
            .mode("#papo")
            .add('m')
            .add_with('k', "bad\r\nOPER")
            .await,
        Err(Error::InvalidInput(crate::InvalidInput::InvalidMessage(_)))
    ));
    let mut byte = [0];
    assert!(
        tokio::time::timeout(Duration::from_millis(10), server.read(&mut byte))
            .await
            .is_err()
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn mode_changes_return_confirmed_facts_after_a_later_disconnect() {
    let (mut server, client, mut events, driver) =
        scripted(config().wanted_caps(["labeled-response"]));
    let driver = tokio::spawn(driver);
    complete_connect_labelled(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver CHANMODES=beI,k,l,imnst MODES=1 :are supported\r\n",
    )
    .await;

    let modes = tokio::spawn({
        let client = client.clone();
        async move { client.mode("#papo").add('m').add('n').await }
    });
    client_line(&mut server, b"@label=0 MODE #papo +m\r\n").await;
    server_line(
        &mut server,
        b"@label=0 :papo-driver!~papo@localhost MODE #papo +m\r\n",
    )
    .await;
    client_line(&mut server, b"@label=1 MODE #papo +n\r\n").await;
    within(server.shutdown()).await.unwrap();

    let result = within(modes).await.unwrap().unwrap();
    assert!(matches!(result.applied.as_slice(), [change] if change.mode == 'm'));
    assert!(result.refused.is_empty());
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
#[allow(clippy::too_many_lines)]
async fn typed_requests_match_unlabelled_responses() {
    let (mut server, client, mut events, driver) =
        scripted(config().rate_limit(papo_core::RateLimit::unlimited()));
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver PREFIX=(ov)@+ CHANMODES=beI,k,l,imnst MODES=3 :are supported\r\n",
    )
    .await;

    let whois = tokio::spawn({
        let client = client.clone();
        async move { client.whois(test_nick("alice")).await }
    });
    client_line(&mut server, b"WHOIS alice\r\n").await;
    server_line(&mut server, b":ergo.test 311 papo-driver alice ~alice host * :Alice Example\r\n:ergo.test 318 papo-driver alice :End of /WHOIS list\r\n").await;
    assert_eq!(within(whois).await.unwrap().unwrap().nick, "alice");

    let who = tokio::spawn({
        let client = client.clone();
        async move { client.who(test_mask("#papo")).await }
    });
    client_line(&mut server, b"WHO #papo\r\n").await;
    server_line(&mut server, b":ergo.test 352 papo-driver #papo ~alice host ergo.test alice H@ :0 Alice Example\r\n:ergo.test 315 papo-driver #papo :End of WHO list\r\n").await;
    assert_eq!(within(who).await.unwrap().unwrap()[0].nick, "alice");

    let join = tokio::spawn({
        let client = client.clone();
        async move { client.join(test_channel("#papo")).await }
    });
    client_line(&mut server, b"JOIN #papo\r\n").await;
    server_line(&mut server, b":papo-driver!papo@localhost JOIN #papo * :Papo Client\r\n:ergo.test 353 papo-driver = #papo :@papo-driver alice\r\n:ergo.test 366 papo-driver #papo :End of /NAMES list\r\n").await;
    within(join).await.unwrap().unwrap();
    assert_eq!(
        client.state().channel("#papo").unwrap().members().count(),
        2
    );
    client_line(&mut server, b"MODE #papo\r\n").await;
    client_line(&mut server, b"WHO #papo\r\n").await;

    let list = tokio::spawn({
        let client = client.clone();
        async move { client.list(crate::ListFilter::default()).await }
    });
    client_line(&mut server, b"LIST\r\n").await;
    server_line(&mut server, b":ergo.test 322 papo-driver #papo 42 :Ergonomic IRC\r\n:ergo.test 323 papo-driver :End of /LIST\r\n").await;
    assert_eq!(within(list).await.unwrap().unwrap()[0].members_count, 42);

    let mode = tokio::spawn({
        let client = client.clone();
        async move { client.mode("#papo").add('t').await }
    });
    client_line(&mut server, b"MODE #papo +t\r\nPING :papo-mode-4\r\n").await;
    server_line(&mut server, b":ergo.test PONG ergo.test :papo-mode-4\r\n").await;
    within(mode).await.unwrap().unwrap();

    let topic = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .set_topic(test_channel("#papo"), "Ergonomic IRC")
                .await
        }
    });
    client_line(&mut server, b"TOPIC #papo :Ergonomic IRC\r\n").await;
    server_line(
        &mut server,
        b":papo-driver!papo@localhost TOPIC #papo :Ergonomic IRC\r\n",
    )
    .await;
    within(topic).await.unwrap().unwrap();

    assert!(matches!(
        client.set_topic(test_channel("#papo"), "").await,
        Err(Error::InvalidInput(crate::InvalidInput::EmptyText))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );

    let clear_topic = tokio::spawn({
        let client = client.clone();
        async move { client.clear_topic(test_channel("#papo")).await }
    });
    client_line(&mut server, b"TOPIC #papo :\r\n").await;
    server_line(
        &mut server,
        b":papo-driver!papo@localhost TOPIC #papo :\r\n",
    )
    .await;
    within(clear_topic).await.unwrap().unwrap();

    let part = tokio::spawn({
        let client = client.clone();
        async move { client.part(test_channel("#papo"), "").await }
    });
    client_line(&mut server, b"PART #papo\r\n").await;
    server_line(
        &mut server,
        b":papo-driver!papo@localhost PART #papo :Leaving\r\n",
    )
    .await;
    within(part).await.unwrap().unwrap();

    let denied = tokio::spawn({
        let client = client.clone();
        async move { client.join(test_channel("#invite-only")).await }
    });
    client_line(&mut server, b"JOIN #invite-only\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 473 papo-driver #invite-only :Cannot join channel (+i)\r\n",
    )
    .await;
    assert!(matches!(
        within(denied).await.unwrap(),
        Err(Error::Server(_))
    ));

    assert!(matches!(
        client
            .request_raw(crate::Line::new("VERSION", Vec::<String>::new()))
            .await,
        Err(Error::Unsupported(crate::Unsupported {
            feature: crate::Feature::LabeledResponse,
            reason: crate::UnsupportedReason::NotOffered,
            ..
        }))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn manual_who_refreshes_joined_member_state_at_numeric_315() {
    let (mut server, client, mut events, driver) = scripted(
        config()
            .rate_limit(papo_core::RateLimit::unlimited())
            .who_on_join(crate::WhoOnJoin::Never),
    );
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver WHOX PREFIX=(ov)@+ :are supported\r\n:papo-driver!papo@localhost JOIN #papo * :Papo Client\r\n:ergo.test 353 papo-driver = #papo :@papo-driver +dan\r\n:ergo.test 366 papo-driver #papo :End of /NAMES list\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Joined(_))));
    assert!(matches!(events.next().await, Some(Event::ChannelSynced(_))));
    client_line(&mut server, b"MODE #papo\r\n").await;

    let mut who = tokio::spawn({
        let client = client.clone();
        async move { client.who(test_channel("#papo")).await }
    });
    client_line(&mut server, b"WHO #papo %tcuhnfar,152\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 354 papo-driver 152 #papo ~dan account/dan dan G+ dan :Daniel Garcia\r\n",
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(1), &mut who)
            .await
            .is_err()
    );
    server_line(
        &mut server,
        b":ergo.test 315 papo-driver #papo :End of WHO list\r\n",
    )
    .await;
    assert_eq!(within(who).await.unwrap().unwrap()[0].nick, "dan");
    assert!(matches!(
        events.next().await,
        Some(Event::MembersRefreshed(refreshed)) if refreshed.channel == "#papo"
    ));
    let state = client.state();
    let dan = state.user("dan").unwrap();
    assert!(matches!(dan.account(), crate::Account::LoggedIn(account) if account == "dan"));
    assert_eq!(dan.away(), &crate::Away::Away { message: None });

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn capability_loss_fails_a_pending_request_immediately() {
    let (mut server, client, mut events, driver) =
        scripted(config().wanted_caps(["labeled-response"]));
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :labeled-response\r\n").await;
    client_line(&mut server, b"CAP REQ :labeled-response\r\n").await;
    server_line(&mut server, b":ergo.test CAP * ACK :labeled-response\r\n").await;
    client_line(&mut server, b"CAP END\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :No MOTD\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Connected(_))));

    let request = tokio::spawn(async move {
        client
            .request_raw(crate::Line::new("VERSION", Vec::<String>::new()))
            .await
    });
    client_line(&mut server, b"@label=0 VERSION\r\n").await;
    server_line(&mut server, b":ergo.test CAP * DEL :labeled-response\r\n").await;
    assert!(matches!(
        within(request).await.unwrap(),
        Err(Error::Unsupported(crate::Unsupported {
            feature: crate::Feature::LabeledResponse,
            reason: crate::UnsupportedReason::NotOffered,
            ..
        }))
    ));
    abort_task(task).await;
}

#[tokio::test(flavor = "current_thread")]
async fn action_confirmation_is_enqueued_after_state_changes() {
    let (mut server, client, mut events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver CHANMODES=Ibe,k,fl,CEMRUimnstu :are supported\r\n",
    )
    .await;
    server_line(&mut server, b":papo-driver!papo@localhost JOIN #papo\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Joined(_))));

    let mode = tokio::spawn({
        let client = client.clone();
        async move { client.mode("#papo").add('t').await }
    });
    client_line(&mut server, b"MODE #papo +t\r\nPING :papo-mode-0\r\n").await;
    server_line(
        &mut server,
        b":papo-driver!papo@localhost MODE #papo +t\r\n:ergo.test PONG ergo.test :papo-mode-0\r\n",
    )
    .await;
    within(mode).await.unwrap().unwrap();

    assert_eq!(events.receiver.len(), 1);
    assert_eq!(
        client.state().channel("#papo").unwrap().mode('t'),
        Some(None)
    );
    assert!(matches!(events.next().await, Some(Event::ModeChanged(_))));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn keyed_join_redacts_the_key_and_waits_for_channel_sync() {
    let (mut server, client, _events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    let join = client
        .join(test_channel("#ergo-ops"))
        .key("unique-dynamic-channel-key-4862");
    let debug = format!("{join:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("unique-dynamic-channel-key-4862"));

    let join = tokio::spawn(async move { join.await });
    client_line(
        &mut server,
        b"JOIN #ergo-ops unique-dynamic-channel-key-4862\r\n",
    )
    .await;
    server_line(&mut server, b":papo-driver!papo@localhost JOIN #ergo-ops * :Papo Client\r\n:ergo.test 353 papo-driver = #ergo-ops :@papo-driver alice\r\n:ergo.test 366 papo-driver #ergo-ops :End of /NAMES list\r\n").await;
    within(join).await.unwrap().unwrap();
    client_line(&mut server, b"MODE #ergo-ops\r\n").await;
    client_line(&mut server, b"WHO #ergo-ops\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn text_only_commands_reject_byte_secrets() {
    let (mut server, client, _events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;

    assert!(matches!(
        client
            .join(test_channel("#ergo-ops"))
            .key(crate::Secret::from_bytes([0xff]))
            .await,
        Err(Error::InvalidInput(crate::InvalidInput::BinarySecret))
    ));
    assert!(matches!(
        client.oper("papo", crate::Secret::from_bytes([0xff])).await,
        Err(Error::InvalidInput(crate::InvalidInput::BinarySecret))
    ));
    abort_task(driver).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn oper_and_ping_return_server_outcomes() {
    let (mut server, client, mut events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;

    let oper = tokio::spawn({
        let client = client.clone();
        async move { client.oper("papo", "unique-oper-password-4928").await }
    });
    client_line(&mut server, b"OPER papo unique-oper-password-4928\r\n").await;
    server_line(&mut server, b":ergo.test 464 * :Password incorrect\r\n").await;
    assert!(matches!(
        within(oper).await.unwrap(),
        Err(Error::Server(error)) if error.code() == "464"
    ));

    let ping = tokio::spawn({
        let client = client.clone();
        async move { client.ping().await }
    });
    client_line(&mut server, b"PING papo-lag-0\r\n").await;
    tokio::time::advance(Duration::from_millis(37)).await;
    server_line(&mut server, b":ergo.test PONG ergo.test :papo-lag-0\r\n").await;
    assert_eq!(
        within(ping).await.unwrap().unwrap(),
        Duration::from_millis(37)
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn labelled_join_timeout_kick_and_part_end_the_action() {
    let settings = config()
        .wanted_caps(["labeled-response"])
        .request_timeout(Duration::from_secs(1));
    let (mut server, client, mut events, driver) = scripted(settings);
    let driver = tokio::spawn(driver);
    complete_connect_labelled(&mut server, &mut events).await;

    let stalled = tokio::spawn({
        let client = client.clone();
        async move { client.join(test_channel("#stalled")).await }
    });
    client_line(&mut server, b"@label=0 JOIN #stalled\r\n").await;
    server_line(
        &mut server,
        b"@label=0 :papo-driver!papo@localhost JOIN #stalled * :papo driver test\r\n",
    )
    .await;
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        within(stalled).await.unwrap(),
        Err(Error::Timeout)
    ));

    let kicked = tokio::spawn({
        let client = client.clone();
        async move { client.join(test_channel("#limited")).await }
    });
    client_line(&mut server, b"@label=1 JOIN #limited\r\n").await;
    server_line(&mut server, b"@label=1 :papo-driver!papo@localhost JOIN #limited * :papo driver test\r\n:ergo.test KICK #limited papo-driver :Channel is full\r\n").await;
    assert!(matches!(
        within(kicked).await.unwrap(),
        Err(Error::UnexpectedReply { command }) if command == "JOIN"
    ));

    let parted = tokio::spawn({
        let client = client.clone();
        async move { client.join(test_channel("#registered")).await }
    });
    client_line(&mut server, b"@label=2 JOIN #registered\r\n").await;
    server_line(&mut server, b"@label=2 :papo-driver!papo@localhost JOIN #registered * :papo driver test\r\n:papo-driver!papo@localhost PART #registered :Registration required\r\n").await;
    assert!(matches!(
        within(parted).await.unwrap(),
        Err(Error::UnexpectedReply { command }) if command == "JOIN"
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
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
    assert_reply_wire(
        true,
        ":ergo.test 005 papo-driver CHANTYPES=# STATUSMSG=@+ :are supported\r\n",
        b"@msgid=RfcBTzyxY120003 :emily!e@localhost PRIVMSG @#ops :!ping\r\n",
        b"@+reply=RfcBTzyxY120003 PRIVMSG @#ops pong\r\n",
        ReplyKind::Reply,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn chat_text_uses_plain_splitting_and_rejects_empty_input() {
    let (mut server, client, _events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    let sent = within(client.privmsg("#papo", "a".repeat(600)))
        .await
        .unwrap();
    assert!(
        matches!(sent.conversation, crate::Conversation::Channel(ref channel) if channel == "#papo")
    );
    assert_eq!(sent.status, None);
    assert_eq!(sent.fallbacks, [crate::Fallback::SplitIntoLines]);
    let lines = format!(
        "PRIVMSG #papo {}\r\nPRIVMSG #papo {}\r\n",
        "a".repeat(413),
        "a".repeat(187)
    );
    client_line(&mut server, lines.as_bytes()).await;
    client.notice("helper", "maintenance soon").await.unwrap();
    client_line(&mut server, b"NOTICE helper :maintenance soon\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver CHANTYPES=# STATUSMSG=@+ :are supported\r\n",
    )
    .await;
    while client.state().isupport().statusmsg() != Some("@+") {
        tokio::task::yield_now().await;
    }
    let sent = client.privmsg("@#ops", "operators only").await.unwrap();
    client_line(&mut server, b"PRIVMSG @#ops :operators only\r\n").await;
    assert!(
        matches!(sent.conversation, crate::Conversation::Channel(ref channel) if channel == "#ops")
    );
    assert_eq!(sent.status, Some(vec!['@']));
    let sent = client
        .privmsg("helper", "reviewed")
        .reply_to(crate::MessageId::from("RfcBTzyxY120004"))
        .channel_context("#papo")
        .await
        .unwrap();
    assert_eq!(
        sent.fallbacks,
        [
            crate::Fallback::ReplyTagDropped,
            crate::Fallback::ContextTagDropped,
        ]
    );
    client_line(&mut server, b"PRIVMSG helper reviewed\r\n").await;
    assert!(matches!(
        within(client.privmsg("#papo", "\n")).await,
        Err(Error::InvalidInput(crate::InvalidInput::EmptyText))
    ));
    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn chat_text_uses_multiline_when_negotiated() {
    let config = config().wanted_caps(["batch", "draft/multiline"]);
    let (mut server, client, _events, driver) = scripted(config);
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(
        &mut server,
        b":ergo.test CAP * LS :batch draft/multiline=max-bytes=4096,max-lines=100\r\n",
    )
    .await;
    client_line(&mut server, b"CAP REQ :batch draft/multiline\r\n").await;
    server_line(
        &mut server,
        b":ergo.test CAP * ACK :batch draft/multiline\r\n",
    )
    .await;
    client_line(&mut server, b"CAP END\r\n").await;
    within(client.privmsg("#papo", "a".repeat(600)))
        .await
        .unwrap();
    let batch = format!(
        "BATCH +b0-0 draft/multiline #papo\r\n@batch=b0-0 PRIVMSG #papo {}\r\n@batch=b0-0;draft/multiline-concat PRIVMSG #papo {}\r\nBATCH -b0-0\r\n",
        "a".repeat(413),
        "a".repeat(187)
    );
    client_line(&mut server, batch.as_bytes()).await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn message_builder_sends_reply_context_and_custom_tags() {
    let config = config().wanted_caps(["echo-message", "labeled-response", "message-tags"]);
    let (mut server, client, mut events, driver) = scripted(config);
    let task = tokio::spawn(driver);
    complete_connect_caps(
        &mut server,
        &mut events,
        "echo-message labeled-response message-tags",
        "echo-message labeled-response message-tags",
    )
    .await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver CLIENTTAGDENY=*,-draft/reply,-channel-context,-example.org/trace :are supported\r\n",
    )
    .await;

    let sender = client.clone();
    let sent = tokio::spawn(async move {
        sender
            .privmsg("helper", "fixed in 69876ec")
            .reply_to("abc123")
            .channel_context(crate::Casemapping::default().channel("#papo"))
            .tag("+example.org/trace", "98")
            .await
    });
    client_line(
        &mut server,
        b"@+example.org/trace=98;+draft/reply=abc123;+channel-context=#papo;label=0 PRIVMSG helper :fixed in 69876ec\r\n",
    )
    .await;
    server_line(
        &mut server,
        b"@label=0;msgid=RfcBTzyxY120005 :papo-driver!papo@localhost PRIVMSG helper :fixed in 69876ec\r\n",
    )
    .await;
    assert_eq!(
        within(sent).await.unwrap().unwrap().msgid.unwrap(),
        "RfcBTzyxY120005"
    );

    let action_client = client.clone();
    let action = tokio::spawn(async move { action_client.action("#papo", "waves").await });
    client_line(
        &mut server,
        b"@label=1 PRIVMSG #papo :\x01ACTION waves\x01\r\n",
    )
    .await;
    server_line(
        &mut server,
        b"@label=1;msgid=RfcBTzyxY120006 :papo-driver!papo@localhost PRIVMSG #papo :\x01ACTION waves\x01\r\n",
    )
    .await;
    assert_eq!(
        within(action).await.unwrap().unwrap().msgid.unwrap(),
        "RfcBTzyxY120006"
    );
    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn message_tag_without_client_prefix_is_invalid_input() {
    let (mut server, client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;

    assert!(matches!(
        within(
            client
                .privmsg("#papo", "deployment complete")
                .tag("account", "alice")
        )
        .await,
        Err(Error::InvalidInput(crate::InvalidInput::InvalidMessage(_)))
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn message_tag_requires_server_support() {
    let (mut server, client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;

    assert!(matches!(
        within(
            client
                .privmsg("#papo", "deployment complete")
                .tag("+example.org/trace", "98")
        )
        .await,
        Err(Error::Unsupported(crate::Unsupported {
            feature: crate::Feature::MessageTags,
            reason: crate::UnsupportedReason::NotOffered,
            ..
        }))
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn message_tag_reports_client_disabled_support() {
    let config = config().without_caps(["message-tags"]);
    let (mut server, client, _events, driver) = scripted(config);
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :message-tags\r\n").await;
    client_line(&mut server, b"CAP END\r\n").await;

    assert!(matches!(
        within(
            client
                .privmsg("#papo", "deployment complete")
                .tag("+example.org/trace", "98")
        )
        .await,
        Err(Error::Unsupported(crate::Unsupported {
            feature: crate::Feature::MessageTags,
            reason: crate::UnsupportedReason::DisabledByClient,
            ..
        }))
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn message_tag_obeys_clienttagdeny() {
    let config = config().wanted_caps(["message-tags"]);
    let (mut server, client, mut events, driver) = scripted(config);
    let task = tokio::spawn(driver);
    complete_connect_caps(&mut server, &mut events, "message-tags", "message-tags").await;
    server_line(
        &mut server,
        b":irc.solanum.example 005 papo-driver CLIENTTAGDENY=example.org/trace :are supported\r\n",
    )
    .await;
    while !client
        .state()
        .isupport()
        .clienttagdeny()
        .denies("example.org/trace")
    {
        tokio::task::yield_now().await;
    }

    assert!(matches!(
        within(
            client
                .privmsg("#papo", "deployment complete")
                .tag("+example.org/trace", "98")
        )
        .await,
        Err(Error::Unsupported(crate::Unsupported {
            feature: crate::Feature::MessageTags,
            reason: crate::UnsupportedReason::TagDenied,
            ..
        }))
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn chat_text_returns_metadata_from_every_labelled_echo() {
    let config = config().wanted_caps(["echo-message", "labeled-response"]);
    let (mut server, client, _events, driver) = scripted(config);
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(
        &mut server,
        b":ergo.test CAP * LS :echo-message labeled-response\r\n",
    )
    .await;
    client_line(&mut server, b"CAP REQ :echo-message labeled-response\r\n").await;
    server_line(
        &mut server,
        b":ergo.test CAP * ACK :echo-message labeled-response\r\n",
    )
    .await;
    client_line(&mut server, b"CAP END\r\n").await;

    let sender = client.clone();
    let sent = tokio::spawn(async move { sender.privmsg("#papo", "hello everyone").await });
    client_line(&mut server, b"@label=0 PRIVMSG #papo :hello everyone\r\n").await;
    server_line(
        &mut server,
        b"@label=0;msgid=RfcBTzyxY120004;time=2026-02-03T04:05:06.789Z :papo-driver!papo@localhost PRIVMSG #papo :hello everyone\r\n",
    )
    .await;
    let sent = within(sent).await.unwrap().unwrap();
    assert_eq!(sent.msgid.as_deref(), Some("RfcBTzyxY120004"));
    assert!(
        matches!(sent.conversation, crate::Conversation::Channel(ref channel) if channel == "#papo")
    );
    assert_eq!(sent.status, None);
    assert_eq!(
        sent.server_time,
        Some(UNIX_EPOCH + Duration::new(1_770_091_506, 789_000_000))
    );

    let sender = client.clone();
    let split = tokio::spawn(async move { sender.notice("#papo", "a".repeat(600)).await });
    let lines = format!(
        "@label=1 NOTICE #papo {}\r\n@label=1-1 NOTICE #papo {}\r\n",
        "a".repeat(414),
        "a".repeat(186)
    );
    client_line(&mut server, lines.as_bytes()).await;
    server_line(
        &mut server,
        b"@label=1;msgid=RfcBTzyxY120005 :papo-driver!papo@localhost NOTICE #papo :first part\r\n",
    )
    .await;
    assert!(!split.is_finished());
    server_line(
        &mut server,
        b"@label=1-1;msgid=RfcBTzyxY120006 :papo-driver!papo@localhost NOTICE #papo :second part\r\n",
    )
    .await;
    let split = within(split).await.unwrap().unwrap();
    assert_eq!(split.line_msgids, ["RfcBTzyxY120005", "RfcBTzyxY120006"]);

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn written_labelled_privmsg_is_interrupted_by_the_server_disconnect() {
    let config = config().wanted_caps(["echo-message", "labeled-response"]);
    let (mut server, client, _events, driver) = scripted(config);
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(
        &mut server,
        b":ergo.test CAP * LS :echo-message labeled-response\r\n",
    )
    .await;
    client_line(&mut server, b"CAP REQ :echo-message labeled-response\r\n").await;
    server_line(
        &mut server,
        b":ergo.test CAP * ACK :echo-message labeled-response\r\n",
    )
    .await;
    client_line(&mut server, b"CAP END\r\n").await;

    let sent = tokio::spawn(async move { client.privmsg("#papo", "deploy complete").await });
    client_line(&mut server, b"@label=0 PRIVMSG #papo :deploy complete\r\n").await;
    server_line(
        &mut server,
        b"ERROR :Closing Link: papo-driver (Server maintenance)\r\n",
    )
    .await;
    assert!(matches!(
        within(sent).await.unwrap(),
        Err(Error::Interrupted(crate::Disconnected {
            cause: crate::DisconnectCause::Server { message },
            retry_in: None,
        })) if message == "Closing Link: papo-driver (Server maintenance)"
    ));
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn actions_reject_every_ctcp_delimiter_without_panicking() {
    let (_server, client, _events, driver) = scripted(config());
    let task = tokio::spawn(driver);

    for text in ["", "waves\rnow", "waves\nnow", "waves\0now", "waves\x01now"] {
        assert!(matches!(
            within(client.action("#papo", text)).await,
            Err(Error::InvalidInput(crate::InvalidInput::InvalidMessage(_)))
        ));
    }

    task.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn chat_text_returns_a_labelled_numeric_refusal() {
    let settings = config().wanted_caps(["echo-message", "labeled-response"]);
    let (mut server, client, mut events, driver) = scripted(settings);
    let task = tokio::spawn(driver);
    complete_connect_caps(
        &mut server,
        &mut events,
        "echo-message labeled-response",
        "echo-message labeled-response",
    )
    .await;

    let sender = client.clone();
    let sent = tokio::spawn(async move { sender.privmsg("#rust", "hello everyone").await });
    client_line(&mut server, b"@label=0 PRIVMSG #rust :hello everyone\r\n").await;
    server_line(
        &mut server,
        b"@label=0 :ergo.test 404 papo-driver #rust :Cannot send to channel\r\n",
    )
    .await;
    assert!(matches!(
        within(sent).await.unwrap(),
        Err(Error::Server(error)) if error.refusal() == crate::Refusal::CannotSendToChannel
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn reactions_obey_clienttagdeny() {
    assert_reaction_wire(
        "",
        false,
        Some(b"@+reply=RfcBTzyxY120004;+draft/react=\xF0\x9F\x91\x8D TAGMSG @#ops\r\n"),
    )
    .await;
    assert_reaction_wire(
        "",
        true,
        Some(b"@+reply=RfcBTzyxY120004;+draft/unreact=\xF0\x9F\x91\x8D TAGMSG @#ops\r\n"),
    )
    .await;
    assert_reaction_wire(
        ":ergo.test 005 papo-driver CLIENTTAGDENY=draft/react :are supported\r\n",
        false,
        None,
    )
    .await;
    assert_reaction_wire(
        ":ergo.test 005 papo-driver CLIENTTAGDENY=reply,draft/reply :are supported\r\n",
        false,
        None,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn reactions_reject_empty_and_oversized_text() {
    let (_server, client, _events, driver) = scripted(config());
    let task = tokio::spawn(driver);

    assert!(matches!(
        within(client.react(("#papo", "RfcBTzyxY120004"), "")).await,
        Err(Error::InvalidInput(crate::InvalidInput::EmptyText))
    ));
    assert!(matches!(
        within(client.react(("#papo", "RfcBTzyxY120004"), "x".repeat(4095))).await,
        Err(Error::InvalidInput(crate::InvalidInput::Reaction {
            maximum: 4094,
            actual: 4095,
        }))
    ));
    assert!(matches!(
        within(client.react(("#papo", "RfcBTzyxY120004"), "x".repeat(4094))).await,
        Err(Error::Unsupported(_))
    ));

    task.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn reactions_accept_own_sent_messages() {
    let caps = ["echo-message", "labeled-response", "message-tags"];
    let (mut server, client, mut events, driver) = scripted(config().wanted_caps(caps));
    let task = tokio::spawn(driver);
    complete_connect_caps(
        &mut server,
        &mut events,
        "echo-message labeled-response message-tags",
        "echo-message labeled-response message-tags",
    )
    .await;

    let sender = client.clone();
    let sent = tokio::spawn(async move { sender.privmsg("#papo", "deploy complete").await });
    client_line(&mut server, b"@label=0 PRIVMSG #papo :deploy complete\r\n").await;
    server_line(
        &mut server,
        b"@label=0;msgid=RfcBTzyxY120004 :papo-driver!papo@localhost PRIVMSG #papo :deploy complete\r\n",
    )
    .await;
    let sent = within(sent).await.unwrap().unwrap();

    let reactor = client.clone();
    let reacted = tokio::spawn(async move { reactor.react(&sent, "👍").await });
    client_line(
        &mut server,
        b"@+reply=RfcBTzyxY120004;+draft/react=\xF0\x9F\x91\x8D;label=1 TAGMSG #papo\r\n",
    )
    .await;
    server_line(
        &mut server,
        b"@label=1;+reply=RfcBTzyxY120004;+draft/react=\xF0\x9F\x91\x8D :papo-driver!papo@localhost TAGMSG #papo\r\n",
    )
    .await;
    within(reacted).await.unwrap().unwrap();

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn reactions_reject_messages_without_identifiers_before_writing() {
    let (mut server, client, mut events, driver) = scripted(config().wanted_caps(["message-tags"]));
    let task = tokio::spawn(driver);
    complete_connect_caps(&mut server, &mut events, "message-tags", "message-tags").await;
    server_line(
        &mut server,
        b":emily!~emily@localhost PRIVMSG #papo :ship it\r\n",
    )
    .await;
    let message = next_message(&mut events).await;

    assert!(matches!(
        within(client.react(&message, "👍")).await,
        Err(Error::InvalidInput(
            crate::InvalidInput::UnusableMessageReference
        ))
    ));
    assert!(matches!(
        within(client.redact(&message, "superseded")).await,
        Err(Error::InvalidInput(
            crate::InvalidInput::UnusableMessageReference
        ))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn typing_throttles_active_per_target_and_sends_terminal_states() {
    let (mut server, client, _events, driver) = scripted(config().wanted_caps(["message-tags"]));
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :message-tags\r\n").await;
    client_line(&mut server, b"CAP REQ :message-tags\r\n").await;
    server_line(&mut server, b":ergo.test CAP * ACK :message-tags\r\n").await;
    client_line(&mut server, b"CAP END\r\n").await;

    within(client.typing("#papo", Typing::Active))
        .await
        .unwrap();
    client_line(&mut server, b"@+typing=active TAGMSG #papo\r\n").await;
    within(client.typing("#PAPO", Typing::Active))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );
    tokio::time::advance(Duration::from_secs(2)).await;
    within(client.typing("#papo", Typing::Active))
        .await
        .unwrap();
    within(client.typing("#papo", Typing::Paused))
        .await
        .unwrap();
    within(client.typing("#papo", Typing::Done)).await.unwrap();
    client_line(
        &mut server,
        b"@+typing=paused TAGMSG #papo\r\n@+typing=done TAGMSG #papo\r\n",
    )
    .await;
    tokio::time::advance(Duration::from_secs(1)).await;
    within(client.typing("#papo", Typing::Active))
        .await
        .unwrap();
    client_line(&mut server, b"@+typing=active TAGMSG #papo\r\n").await;

    server_line(
        &mut server,
        b":ergo.test 005 papo-driver CLIENTTAGDENY=typing :are supported\r\n",
    )
    .await;
    while !client.state().isupport().clienttagdeny().denies("typing") {
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        within(client.typing("emily", Typing::Done)).await,
        Err(Error::Unsupported(crate::Unsupported {
            feature: crate::Feature::Typing,
            reason: crate::UnsupportedReason::TagDenied,
            ..
        }))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn typing_state_appears_refreshes_without_a_snapshot_and_expires() {
    let (mut server, client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;

    server_line(
        &mut server,
        b"@+typing=active :alice!~alice@cloak.example TAGMSG #papo\r\n",
    )
    .await;
    let conversation = match events.next().await {
        Some(Event::TypingChanged(change)) => change.conversation,
        other => panic!("expected typing event, got {other:?}"),
    };
    let state = client.state();
    let typer = state.typing(&conversation).next().unwrap();
    assert_eq!(typer.nick, "alice");
    assert_eq!(typer.state, Typing::Active);
    assert_eq!(typer.status, None);

    tokio::time::advance(Duration::from_secs(3)).await;
    server_line(
        &mut server,
        b"@+typing=active :ALICE!~alice@cloak.example TAGMSG #papo\r\n",
    )
    .await;
    assert!(matches!(
        events.next().await,
        Some(Event::TypingChanged(change)) if change.state == Typing::Active
    ));
    assert!(Arc::ptr_eq(&state, &client.state()));

    tokio::time::advance(Duration::from_secs(6)).await;
    assert!(matches!(
        events.next().await,
        Some(Event::TypingChanged(change)) if change.state == Typing::Done
    ));
    let expired = client.state();
    assert!(!Arc::ptr_eq(&state, &expired));
    assert_eq!(expired.typing(&conversation).count(), 0);

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn typing_events_report_every_transition_reason() {
    use crate::event::TypingReason;

    let (mut server, _client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;

    server_line(
        &mut server,
        b"@+typing=active :alice!~alice@cloak.example TAGMSG #papo\r\n",
    )
    .await;
    assert!(matches!(
        events.next().await,
        Some(Event::TypingChanged(change)) if change.reason == TypingReason::Notified
    ));
    server_line(
        &mut server,
        b":alice!~alice@cloak.example PRIVMSG #papo :the build is green\r\n",
    )
    .await;
    assert!(matches!(
        events.next().await,
        Some(Event::TypingChanged(change)) if change.reason == TypingReason::Messaged
    ));
    assert!(matches!(events.next().await, Some(Event::Message(_))));

    server_line(
        &mut server,
        b"@+typing=paused :bob!~bob@users.example TAGMSG #papo\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::TypingChanged(_))));
    server_line(
        &mut server,
        b":bob!~bob@users.example PART #papo :Leaving\r\n",
    )
    .await;
    assert!(matches!(
        events.next().await,
        Some(Event::TypingChanged(change)) if change.reason == TypingReason::Departed
    ));
    assert!(matches!(events.next().await, Some(Event::Parted(_))));

    server_line(
        &mut server,
        b"@+typing=active :carol!~carol@users.example TAGMSG #papo\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::TypingChanged(_))));
    tokio::time::advance(Duration::from_secs(6)).await;
    assert!(matches!(
        events.next().await,
        Some(Event::TypingChanged(change)) if change.reason == TypingReason::TimedOut
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn redaction_requires_its_capability_and_propagates_fail() {
    let caps = ["draft/message-redaction", "labeled-response"];
    let (mut server, client, _events, driver) = scripted(config().wanted_caps(caps));
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(
        &mut server,
        b":ergo.test CAP * LS :draft/message-redaction labeled-response\r\n",
    )
    .await;
    client_line(
        &mut server,
        b"CAP REQ :draft/message-redaction labeled-response\r\n",
    )
    .await;
    server_line(
        &mut server,
        b":ergo.test CAP * ACK :draft/message-redaction labeled-response\r\n",
    )
    .await;
    client_line(&mut server, b"CAP END\r\n").await;

    let sender = client.clone();
    let redacted = tokio::spawn(async move {
        sender
            .redact(
                ("#papo", crate::MessageId::from("RfcBTzyxY120004")),
                "duplicate message",
            )
            .await
    });
    client_line(
        &mut server,
        b"@label=0 REDACT #papo RfcBTzyxY120004 :duplicate message\r\n",
    )
    .await;
    server_line(
        &mut server,
        b"@label=0 :papo-driver!papo@localhost REDACT #papo RfcBTzyxY120004 :duplicate message\r\n",
    )
    .await;
    assert!(within(redacted).await.unwrap().is_ok());

    let sender = client.clone();
    let rejected = tokio::spawn(async move {
        sender
            .redact(("#papo", crate::MessageId::from("RfcBTzyxY120005")), "")
            .await
    });
    client_line(&mut server, b"@label=1 REDACT #papo RfcBTzyxY120005\r\n").await;
    server_line(
        &mut server,
        b"@label=1 :ergo.test FAIL REDACT FORBIDDEN #papo :Insufficient privileges\r\n",
    )
    .await;
    assert!(matches!(
        within(rejected).await.unwrap(),
        Err(Error::Server(error)) if error.code() == "FORBIDDEN"
    ));

    server_line(
        &mut server,
        b":ergo.test CAP * DEL :draft/message-redaction\r\n",
    )
    .await;
    while client
        .state()
        .caps()
        .contains_key(&crate::cap::MESSAGE_REDACTION)
    {
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        within(client.redact(("#papo", crate::MessageId::from("RfcBTzyxY120006")), "")).await,
        Err(Error::Unsupported(crate::Unsupported {
            feature: crate::Feature::Redaction,
            reason: crate::UnsupportedReason::NotOffered,
            ..
        }))
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn labelled_echo_is_consumed_before_an_own_live_line() {
    let caps = ["echo-message", "labeled-response"];
    let (mut server, client, mut events, driver) = scripted(config().wanted_caps(caps));
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(
        &mut server,
        b":ergo.test CAP * LS :echo-message labeled-response\r\n",
    )
    .await;
    client_line(&mut server, b"CAP REQ :echo-message labeled-response\r\n").await;
    server_line(
        &mut server,
        b":ergo.test CAP * ACK :echo-message labeled-response\r\n",
    )
    .await;
    client_line(&mut server, b"CAP END\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :No MOTD\r\n",
    )
    .await;
    while !matches!(events.next().await, Some(Event::Connected(_))) {}

    let sender = client.clone();
    let sent = tokio::spawn(async move { sender.privmsg("#papo", "from this device").await });
    client_line(&mut server, b"@label=0 PRIVMSG #papo :from this device\r\n").await;
    server_line(
        &mut server,
        b"@label=0;msgid=RfcBTzyxY120004 :papo-driver!papo@localhost PRIVMSG #papo :from this device\r\n",
    )
    .await;
    assert!(within(sent).await.unwrap().is_ok());

    server_line(
        &mut server,
        b"@msgid=RfcBTzyxY120005 :papo-driver!papo@localhost PRIVMSG #papo :from another device\r\n",
    )
    .await;
    let other = next_message(&mut events).await;
    assert_eq!(other.origin, Origin::Live);
    assert!(other.is_live());
    assert_eq!(other.authorship, crate::Authorship::Own);

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
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

#[tokio::test(flavor = "current_thread")]
async fn client_send_raw_rejects_invalid_lines() {
    let (mut server, client, _events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    for line in [
        crate::Line::new("", Vec::<String>::new()),
        crate::Line::new("   ", Vec::<String>::new()),
        crate::Line::new("WHOIS", ["emily\r\n"]),
        crate::Line::new("WHOIS", ["emily\nPRIVMSG #papo :injected"]),
        crate::Line::new("PING", ["\0token"]),
        crate::Line::new("KILL", ["alice evil", "reason"]),
        crate::Line::new("KILL", [":x", "reason"]),
        crate::Line::new("KILL", ["", "reason"]),
        crate::Line::new("KILL", ["alice", "reason\nQUIT"]),
        crate::Line::new("KILL", ["alice", "reason\0QUIT"]),
    ] {
        assert!(matches!(
            client.send_raw(line).await,
            Err(Error::InvalidInput(crate::InvalidInput::InvalidMessage(_)))
        ));
    }
    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn quit_resolves_when_the_server_closed_first() {
    let (mut server, client, mut events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
    assert!(client.quit("leaving for maintenance").await.is_ok());
    assert!(matches!(
        client.quit("maintenance\nQUIT").await,
        Err(Error::InvalidInput(crate::InvalidInput::InvalidMessage(_)))
    ));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::ConnectionLost { .. },
            retry_in: None,
        }))
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
    welcome_burst(&mut server).await;
    let quit = tokio::spawn(async move { client.quit("").await });
    client_line(&mut server, b"QUIT\r\n").await;

    read_error.store(true, Ordering::Relaxed);
    within(server.write_all(b":ergo.test NOTICE papo-driver :connection closing\r\n"))
        .await
        .unwrap();

    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Requested {
                quit_unacknowledged: false,
            },
            retry_in: None,
        }))
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn quit_times_out_after_ten_seconds() {
    let (mut server, client, mut events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    let quit = tokio::spawn(async move { client.quit("").await });
    let mut line = [0; 6];
    within(server.read_exact(&mut line)).await.unwrap();
    assert_eq!(&line, b"QUIT\r\n");
    tokio::time::advance(Duration::from_secs(9)).await;
    assert!(!quit.is_finished());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Requested {
                quit_unacknowledged: true,
            },
            retry_in: None,
        }))
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn quit_deadline_bounds_final_flush_for_a_peer_that_never_drains() {
    let (inner, mut server) = tokio::io::duplex(4096);
    let written = Arc::new(Mutex::new(Vec::new()));
    let stream = NeverDrainingStream {
        inner,
        written: Arc::clone(&written),
    };
    let (client, _events, driver) = Driver::with_stream(stream, Session::new(config()));
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    let quit = tokio::spawn(async move { client.quit("bounded shutdown").await });
    while !written
        .lock()
        .unwrap()
        .windows(b"QUIT :bounded shutdown\r\n".len())
        .any(|window| window == b"QUIT :bounded shutdown\r\n")
    {
        tokio::task::yield_now().await;
    }

    tokio::time::advance(Duration::from_secs(10)).await;
    let quit = tokio::time::timeout(Duration::from_secs(1), quit)
        .await
        .expect("final flush exceeded the quit deadline")
        .unwrap();
    assert!(quit.is_ok());
    assert!(
        tokio::time::timeout(Duration::from_secs(1), driver)
            .await
            .expect("driver did not stop after the quit deadline")
            .unwrap()
            .is_ok()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_every_client_sends_quit() {
    let (mut server, client, _events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    drop(client);
    let mut line = [0; 6];
    within(server.read_exact(&mut line)).await.unwrap();
    assert_eq!(&line, b"QUIT\r\n");
    within(server.shutdown()).await.unwrap();
    assert!(within(driver).await.unwrap().is_ok());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn full_event_channel_still_serves_quit_and_its_timer() {
    let (mut server, client, events, driver) = scripted_with_capacity(config(), 1);
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    within(server.write_all(b":ergo.test NOTICE papo-driver :maintenance soon\r\n:ergo.test NOTICE papo-driver :still queued\r\n"))
        .await
        .unwrap();
    while events.receiver.len() != 1 {
        tokio::task::yield_now().await;
    }

    let quit = tokio::spawn(async move { client.quit("").await });
    let mut line = [0; 6];
    within(server.read_exact(&mut line)).await.unwrap();
    assert_eq!(&line, b"QUIT\r\n");
    tokio::time::advance(Duration::from_secs(10)).await;

    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
    assert_eq!(events.receiver.len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn quit_reads_server_close_while_events_are_backed_up() {
    let (mut server, client, events, driver) = scripted_with_capacity(config(), 1);
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    server_line(
        &mut server,
        b":ergo.test NOTICE papo-driver :maintenance soon\r\n:ergo.test NOTICE papo-driver :still queued\r\n",
    )
    .await;
    while events.receiver.len() != 1 {
        tokio::task::yield_now().await;
    }

    let quit = tokio::spawn(async move { client.quit("").await });
    client_line(&mut server, b"QUIT\r\n").await;
    server_line(
        &mut server,
        b"ERROR :Closing Link: papo-driver (Client Quit)\r\n",
    )
    .await;

    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
    assert_eq!(events.receiver.len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn only_pending_request_enables_read_ahead() {
    let (mut server, client, mut events, driver) = scripted_with_capacity(config(), 2);
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    saturate_events(&mut server, &events).await;

    let whois = tokio::spawn(async move { client.whois(test_nick("alice")).await });
    client_line(&mut server, b"WHOIS alice\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 311 papo-driver alice ~alice staff.example * :Alice Example\r\n:ergo.test 318 papo-driver alice :End of /WHOIS list\r\n",
    )
    .await;
    assert_eq!(within(whois).await.unwrap().unwrap().nick, "alice");
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    assert!(matches!(events.next().await, Some(Event::Message(_))));

    abort_task(driver).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn quit_publishes_member_removal_while_reading_ahead() {
    let (mut server, client, mut events, driver) = scripted_with_capacity(config(), 2);
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":papo-driver!papo@localhost JOIN #papo * :papo driver test\r\n:alice!~alice@staff.example JOIN #papo * :Alice Example\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Joined(_))));
    assert!(matches!(events.next().await, Some(Event::Joined(_))));
    assert!(
        client
            .state()
            .channel("#papo")
            .unwrap()
            .member("alice")
            .is_some()
    );
    saturate_events(&mut server, &events).await;

    let requester = client.clone();
    let whois = tokio::spawn(async move { requester.whois(test_nick("dan")).await });
    client_line(&mut server, b"WHOIS dan\r\n").await;
    server_line(
        &mut server,
        b":alice!~alice@staff.example QUIT :Client exited\r\n:ergo.test 311 papo-driver dan ~dan users.example * :Dan Example\r\n:ergo.test 318 papo-driver dan :End of /WHOIS list\r\n",
    )
    .await;
    assert_eq!(within(whois).await.unwrap().unwrap().nick, "dan");

    loop {
        let event = within(events.next())
            .await
            .expect("event stream remains open");
        if matches!(event, Event::Quit(_)) {
            assert!(
                client
                    .state()
                    .channel("#papo")
                    .unwrap()
                    .member("alice")
                    .is_none()
            );
            break;
        }
    }

    abort_task(driver).await;
}

#[test]
fn registration_and_quit_independently_enable_read_ahead() {
    let requests = std::collections::BTreeMap::new();
    let sync_waiters = Vec::new();
    let catchups = std::collections::BTreeMap::new();
    let registration = super::ConnectWait::default();
    assert!(!super::should_read_ahead(
        &requests,
        &sync_waiters,
        &catchups,
        &registration,
        false,
    ));

    let (registration, _result) = tokio::sync::oneshot::channel();
    let registration = super::ConnectWait(Some(registration));
    assert!(super::should_read_ahead(
        &requests,
        &sync_waiters,
        &catchups,
        &registration,
        false,
    ));
    assert!(super::should_read_ahead(
        &requests,
        &sync_waiters,
        &catchups,
        &super::ConnectWait::default(),
        true,
    ));
}

#[test]
fn closed_waiters_do_not_enable_read_ahead() {
    let now = crate::rt::now_mono();
    let mut session = Session::new(config());
    let id = session
        .request_with_matcher(
            now,
            Line::new("WHOIS", ["alice"]),
            papo_core::ResponseMatcher::Whois("alice".into()),
        )
        .unwrap();
    let (request, request_receiver) = tokio::sync::oneshot::channel();
    let mut requests =
        std::collections::BTreeMap::from([(id, super::PendingRequest::Response(request))]);
    let (sync, sync_receiver) = tokio::sync::oneshot::channel();
    let mut sync_waiters = vec![("#papo".to_owned(), sync)];
    let (registration, registration_receiver) = tokio::sync::oneshot::channel();
    let mut registration = super::ConnectWait(Some(registration));
    drop((request_receiver, sync_receiver, registration_receiver));

    super::drop_closed_waiters(&mut requests, &mut sync_waiters, &mut registration);

    assert!(requests.is_empty());
    assert!(sync_waiters.is_empty());
    assert!(registration.0.is_none());
    assert!(!super::should_read_ahead(
        &requests,
        &sync_waiters,
        &std::collections::BTreeMap::new(),
        &registration,
        false,
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn only_channel_sync_waiter_enables_read_ahead() {
    let (mut server, client, mut events, driver) = scripted_with_capacity(config(), 2);
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":papo-driver!papo@localhost JOIN #papo * :papo driver test\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Joined(_))));
    saturate_events(&mut server, &events).await;

    let (result, waiter) = tokio::sync::oneshot::channel();
    client
        .commands
        .send(super::Command::WaitChannelSync {
            channel: "#papo".to_owned(),
            result,
        })
        .await
        .unwrap();
    tokio::task::yield_now().await;
    server_line(
        &mut server,
        b":ergo.test 353 papo-driver = #papo :@papo-driver alice\r\n:ergo.test 366 papo-driver #papo :End of /NAMES list\r\n",
    )
    .await;
    within(waiter).await.unwrap().unwrap();

    abort_task(driver).await;
}

#[tokio::test(flavor = "current_thread")]
async fn pending_read_marker_request_enables_read_ahead() {
    let settings = config().wanted_caps(["draft/read-marker", "labeled-response"]);
    let (mut server, client, mut events, driver) = scripted_with_capacity(settings, 2);
    let driver = tokio::spawn(driver);
    complete_connect_read_marker(&mut server, &mut events).await;
    saturate_events(&mut server, &events).await;

    let marker = tokio::spawn(async move { client.fetch_read_marker("#papo").await });
    client_line(&mut server, b"@label=0 MARKREAD #papo\r\n").await;
    server_line(
        &mut server,
        b"@label=0 :ergo.test MARKREAD #papo timestamp=2025-10-15T11:55:42.123Z\r\n",
    )
    .await;
    assert_eq!(
        within(marker).await.unwrap().unwrap(),
        Some(SystemTime::UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000))
    );

    abort_task(driver).await;
}

#[tokio::test(flavor = "current_thread")]
async fn no_pending_work_disables_read_ahead() {
    let (mut server, _client, mut events, driver) = scripted_with_capacity(config(), 2);
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    saturate_events(&mut server, &events).await;

    server_line(&mut server, b"PING :read-ahead-blocked\r\n").await;
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    client_line(&mut server, b"PONG :read-ahead-blocked\r\n").await;

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Ok(()) | Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn disconnect_preserves_events_waiting_for_channel_capacity() {
    let (mut server, client, mut events, driver) = scripted_with_capacity(config(), 1);
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    let whois = tokio::spawn(async move { client.whois(test_nick("alice")).await });
    client_line(&mut server, b"WHOIS alice\r\n").await;
    server_line(
        &mut server,
        b":ergo.test NOTICE papo-driver :maintenance soon\r\n:ergo.test NOTICE papo-driver :maintenance extended\r\nERROR :Closing Link: papo-driver (Server shutdown)\r\n",
    )
    .await;

    assert!(matches!(
        within(whois).await.unwrap(),
        Err(Error::Interrupted(crate::Disconnected {
            cause: crate::DisconnectCause::Server { .. },
            retry_in: None,
        }))
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
    assert_eq!(next_message(&mut events).await.text, "maintenance soon");
    assert_eq!(next_message(&mut events).await.text, "maintenance extended");
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Server { message },
            retry_in: None,
        })) if message == "Closing Link: papo-driver (Server shutdown)"
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn unread_events_past_the_overflow_bound_disconnect_the_consumer() {
    let (mut server, client, mut events, driver) = scripted_with_capacity(config(), 1);
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    let whois = tokio::spawn(async move { client.whois(test_nick("alice")).await });
    client_line(&mut server, b"WHOIS alice\r\n").await;
    server_line(
        &mut server,
        b":ergo.test NOTICE papo-driver :maintenance one\r\n:ergo.test NOTICE papo-driver :maintenance two\r\n:ergo.test NOTICE papo-driver :maintenance three\r\n",
    )
    .await;

    assert!(matches!(
        within(whois).await.unwrap(),
        Err(Error::Interrupted(crate::Disconnected {
            cause: crate::DisconnectCause::SlowConsumer,
            retry_in: None,
        }))
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::SlowConsumer,
            retry_in: None,
        }))
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn slow_consumer_reconnects_after_its_reported_delay() {
    let (first_stream, mut first_server) = tokio::io::duplex(4096);
    let (second_stream, _second_server) = tokio::io::duplex(4096);
    let streams = Arc::new(Mutex::new(VecDeque::from([
        Box::new(first_stream) as super::BoxConnection,
        Box::new(second_stream) as super::BoxConnection,
    ])));
    let calls = Arc::new(AtomicUsize::new(0));
    let connector: super::Connector = Arc::new({
        let streams = Arc::clone(&streams);
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            let stream = streams.lock().unwrap().pop_front().unwrap();
            Box::pin(async move { Ok(stream) })
        }
    });
    let (client, mut events, driver) = Driver::with_connector_and_entropy(
        Ok(ConnectionPlan {
            address: "irc://irc.example.net".parse().unwrap(),
            tls: TlsConfig::default(),
        }),
        Session::new(config()),
        1,
        None,
        super::PostConnect::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
        Box::new(|| 0),
    );
    let driver = tokio::spawn(driver);
    complete_connect(&mut first_server, &mut events).await;

    let whois = tokio::spawn({
        let client = client.clone();
        async move { client.whois(test_nick("alice")).await }
    });
    client_line(&mut first_server, b"WHOIS alice\r\n").await;
    server_line(
        &mut first_server,
        b":ergo.test NOTICE papo-driver :maintenance one\r\n:ergo.test NOTICE papo-driver :maintenance two\r\n:ergo.test NOTICE papo-driver :maintenance three\r\n",
    )
    .await;

    assert!(matches!(
        within(whois).await.unwrap(),
        Err(Error::Interrupted(crate::Disconnected {
            cause: crate::DisconnectCause::SlowConsumer,
            ..
        }))
    ));
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::SlowConsumer,
            retry_in: Some(delay),
        })) if delay == Duration::from_millis(800)
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    tokio::time::advance(Duration::from_millis(800)).await;
    while calls.load(Ordering::Relaxed) != 2 {
        tokio::task::yield_now().await;
    }
    client.quit("test complete").await.unwrap();
    assert!(within(driver).await.unwrap().is_ok());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn keepalive_expiry_while_events_are_stalled_is_a_slow_consumer() {
    let settings = config()
        .ping_interval(Duration::from_secs(30))
        .ping_timeout(Duration::from_secs(120));
    let (mut server, _client, mut events, driver) = scripted_with_capacity(settings, 1);
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test NOTICE papo-driver :maintenance one\r\n:ergo.test NOTICE papo-driver :maintenance two\r\n",
    )
    .await;
    while events.receiver.len() != 1 {
        tokio::task::yield_now().await;
    }

    tokio::time::advance(Duration::from_secs(30)).await;
    client_line(&mut server, b"PING :papo-0\r\n").await;
    tokio::time::advance(Duration::from_secs(90)).await;
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::SlowConsumer,
            retry_in: None,
        }))
    ));
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::SlowConsumer,
            retry_in: None,
        }))
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn session_deadline_sends_keepalive_ping() {
    let config = config()
        .welcome_timeout(Duration::from_secs(300))
        .ping_interval(Duration::from_secs(30))
        .ping_timeout(Duration::from_secs(120));
    let (mut server, _client, mut events, driver) = scripted(config);
    let task = tokio::spawn(driver);
    welcome_burst(&mut server).await;
    tokio::time::advance(Duration::from_secs(30)).await;
    let mut ping = [0; 14];
    within(server.read_exact(&mut ping)).await.unwrap();
    assert_eq!(&ping, b"PING :papo-0\r\n");
    tokio::time::advance(Duration::from_secs(90)).await;
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::PingTimeout,
            retry_in: None,
        }))
    ));
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn client_reads_owned_state_snapshots() {
    let (mut server, client, mut events, driver) = scripted(config());
    assert!(!client.state().is_connected());
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    assert!(client.state().is_connected());
    assert_eq!(client.state().account(), &crate::Account::LoggedOut);

    server_line(
        &mut server,
        b":papo-driver!papo@localhost JOIN #Papo\r\n:ergo.test 353 papo-driver = #Papo :@papo-driver +emily\r\n:ergo.test 366 papo-driver #Papo :End of NAMES list\r\n",
    )
    .await;
    while !matches!(events.next().await, Some(Event::ChannelSynced(_))) {}
    let state = client.state();
    let channel = state.channel("#papo").unwrap();
    assert!(channel.is_synced());
    assert_eq!(channel.member("EMILY").unwrap().prefixes(), &['+']);

    let unchanged = client.state();
    server_line(
        &mut server,
        b":ergo.test NOTICE papo-driver :maintenance soon\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    assert!(Arc::ptr_eq(&unchanged, &client.state()));

    server_line(
        &mut server,
        b":papo-driver!papo@localhost PART #Papo :leaving\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Parted(_))));
    assert!(!Arc::ptr_eq(&unchanged, &client.state()));
    assert!(client.state().channel("#papo").is_none());

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
    assert!(!client.state().is_connected());
}

#[tokio::test(flavor = "current_thread")]
async fn public_state_accessors_report_the_registered_session() {
    let config = config()
        .wanted_caps(["extended-monitor"])
        .sasl(Sasl::plain("alice", "hunter2"));
    let (mut server, client, mut events, driver) = scripted(config);
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
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
    assert!(matches!(events.next().await, Some(Event::Connected(_))));
    assert_eq!(client.state().away(), &crate::Away::Present);

    server_line(&mut server, b":ergo.test 005 papo-driver CASEMAPPING=ascii CHANTYPES=# PREFIX=(ov)@+ MONITOR=100 NICKLEN=27 BOUNCER_NETID=42 draft/FILEHOST=https://files.example.net/upload :are supported\r\n:papo-driver!papo@localhost MODE papo-driver +iw\r\n:papo-driver!papo@localhost JOIN #Papo\r\n:ergo.test 353 papo-driver = #Papo :@papo-driver alice!~alice@cloak.example\r\n:ergo.test 366 papo-driver #Papo :End of NAMES list\r\n:ergo.test 730 papo-driver :bob!bob@users.example\r\n").await;
    while !matches!(events.next().await, Some(Event::MonitorOnline(_))) {}
    client_line(&mut server, b"MODE #Papo\r\n").await;
    client_line(&mut server, b"WHO #Papo\r\n").await;

    let away = tokio::spawn({
        let client = client.clone();
        async move { client.set_away("reviewing patches").await }
    });
    client_line(&mut server, b"AWAY :reviewing patches\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 306 papo-driver :You have been marked as being away\r\n",
    )
    .await;
    within(away).await.unwrap().unwrap();

    let nick: crate::Nick = client.nick();
    assert_eq!(nick, "papo-driver");
    let state = client.state();
    assert_eq!(state.nick(), &nick);
    assert_eq!(
        state.account(),
        &crate::Account::LoggedIn(test_account("emily"))
    );
    assert_eq!(
        state.away(),
        &crate::Away::Away {
            message: Some("reviewing patches".into())
        }
    );
    assert_eq!(state.host(), Some("localhost"));
    assert_eq!(state.realname(), "papo driver test");
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
    assert_eq!(
        state
            .user(state.casemapping().nick("ALICE"))
            .unwrap()
            .username(),
        Some("~alice")
    );
    assert_eq!(state.monitor_online().next().unwrap().nick(), "bob");
    assert_eq!(state.isupport().nicklen(), Some(27));
    #[cfg(feature = "unstable")]
    {
        assert_eq!(
            state.bouncer_network().map(papo_core::NetworkId::as_str),
            Some("42")
        );
        assert_eq!(state.filehost(), Some("https://files.example.net/upload"));
    }
    assert!(
        state
            .features()
            .any(|feature| feature == papo_core::Feature::Monitor)
    );
    let snapshot = client.state();
    assert_eq!(snapshot.nick(), "papo-driver");
    assert_eq!(
        snapshot.account(),
        &crate::Account::LoggedIn(test_account("emily"))
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
#[allow(clippy::too_many_lines)]
async fn every_state_change_replaces_the_snapshot() {
    let config = config().wanted_caps(["extended-monitor"]);
    let (mut server, client, mut events, driver) = scripted(config);
    let driver = tokio::spawn(driver);
    complete_connect_extended_monitor(&mut server, &mut events).await;
    let mut previous = client.state();

    server_line(&mut server, b":papo-driver!papo@localhost JOIN #Papo\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Joined(_))));
    previous = changed_snapshot(&client, &previous, "JOIN");
    assert!(previous.channel("#papo").is_some());

    server_line(&mut server, b":ergo.test MODE papo-driver +i\r\n").await;
    assert!(matches!(events.next().await, Some(Event::ModeChanged(_))));
    previous = changed_snapshot(&client, &previous, "MODE");
    assert_eq!(previous.user_modes().collect::<Vec<_>>(), ['i']);

    server_line(
        &mut server,
        b":emily!em@staff.example TOPIC #Papo :Release planning\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::TopicChanged(_))));
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
        Some(Event::AccountChanged(_))
    ));
    previous = changed_snapshot(&client, &previous, "ACCOUNT");
    assert_eq!(
        previous.user("papo-driver").unwrap().account(),
        &papo_core::Account::LoggedIn(test_account("emily"))
    );

    server_line(
        &mut server,
        b":papo-driver!papo@localhost AWAY :Gone to lunch\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::AwayChanged(_))));
    previous = changed_snapshot(&client, &previous, "AWAY");
    assert_eq!(
        previous.user("papo-driver").unwrap().away(),
        &crate::Away::Away {
            message: Some("Gone to lunch".into())
        }
    );

    server_line(
        &mut server,
        b":papo-driver!papo@localhost NICK papo-reader\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::NickChanged(_))));
    previous = changed_snapshot(&client, &previous, "NICK");
    assert_eq!(previous.nick(), "papo-reader");

    server_line(
        &mut server,
        b":ergo.test 730 papo-reader :alice!~alice@cloak.example\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::MonitorOnline(_))));
    previous = changed_snapshot(&client, &previous, "MONITOR online");
    assert_eq!(previous.monitor_online().count(), 1);

    server_line(&mut server, b":ergo.test 731 papo-reader :alice\r\n").await;
    assert!(matches!(
        events.next().await,
        Some(Event::MonitorOffline(_))
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
    assert!(matches!(events.next().await, Some(Event::Parted(_))));
    let current = changed_snapshot(&client, &previous, "PART");
    assert!(current.channel("#papo").is_none());

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn client_exposes_capability_aliases_and_casemapping() {
    let config = config().wanted_caps(["draft/no-implicit-names"]);
    let (mut server, client, mut events, driver) = scripted(config);
    let driver = tokio::spawn(driver);
    welcome_burst(&mut server).await;
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
    assert!(matches!(
        events.next().await,
        Some(Event::CasemappingChanged(changed))
            if changed.old == crate::Casemapping::Rfc1459
                && changed.new == crate::Casemapping::Ascii
    ));
    assert!(matches!(events.next().await, Some(Event::Connected(_))));

    let state = client.state();
    assert!(state.caps().contains_key(&crate::cap::NO_IMPLICIT_NAMES));
    assert!(
        state
            .caps()
            .contains_key(&crate::Cap::from("no-implicit-names"))
    );
    assert!(!state.caps().contains_key(&crate::cap::MESSAGE_TAGS));
    assert_eq!(state.isupport().nicklen(), Some(27));
    assert!(state.is_connected());
    assert_eq!(state.nick(), "PAPO-DRIVER");
    assert_eq!(state.channel_name("#Papo"), "#papo");
    assert!(state.channel_name("#PAPO[dev]") != "#papo{dev}");
    assert!(!state.supports(crate::Feature::History));
    assert!(
        !state
            .features()
            .any(|feature| feature == crate::Feature::History)
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn casemapping_change_refolds_the_state_snapshot() {
    let (mut server, client, mut events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":papo-driver!papo@localhost JOIN #Papo[dev]\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Joined(_))));
    let rfc1459 = client.state();
    assert!(rfc1459.channel("#papo{dev}").is_some());

    server_line(
        &mut server,
        b":ergo.test 005 papo-driver CASEMAPPING=ascii :are supported\r\n",
    )
    .await;
    assert!(matches!(
        events.next().await,
        Some(Event::CasemappingChanged(changed))
            if changed.old == crate::Casemapping::Rfc1459
                && changed.new == crate::Casemapping::Ascii
    ));
    let ascii = client.state();
    assert!(!Arc::ptr_eq(&rfc1459, &ascii));
    assert!(ascii.channel("#papo[dev]").is_some());
    assert!(ascii.channel("#papo{dev}").is_none());

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn monitor_offline_snapshot_keeps_other_online_users() {
    let config = config().wanted_caps(["extended-monitor"]);
    let (mut server, client, mut events, driver) = scripted(config);
    let driver = tokio::spawn(driver);
    complete_connect_extended_monitor(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 730 papo-driver :alice!~alice@cloak.example,bob!bob@users.example\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::MonitorOnline(_))));
    assert!(matches!(events.next().await, Some(Event::MonitorOnline(_))));

    server_line(&mut server, b":ergo.test 731 papo-driver :alice\r\n").await;
    assert!(
        matches!(events.next().await, Some(Event::MonitorOffline(offline)) if offline.nick == "alice")
    );
    let online: Vec<_> = client
        .state()
        .monitor_online()
        .map(|user| user.nick().to_owned())
        .collect();
    assert_eq!(online, ["bob"]);

    within(server.shutdown()).await.unwrap();
    _ = within(driver).await;
}

#[tokio::test(flavor = "current_thread")]
async fn disconnect_clears_monitor_presence_snapshot() {
    let config = config().wanted_caps(["extended-monitor"]);
    let (mut server, client, mut events, driver) = scripted(config);
    let driver = tokio::spawn(driver);
    complete_connect_extended_monitor(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 730 papo-driver :alice!~alice@cloak.example\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::MonitorOnline(_))));
    assert_eq!(client.state().monitor_online().count(), 1);

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
    assert_eq!(client.state().monitor_online().count(), 0);
}
