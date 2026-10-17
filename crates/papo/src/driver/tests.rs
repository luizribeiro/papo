use super::test_support::*;

macro_rules! disconnected {
    () => {
        Error::Disconnected(_)
    };
}

fn unavailable(message: &'static str) -> Error {
    Error::connect_unavailable(message)
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
    assert_eq!(next_message(&mut events).await.kind(), MessageKind::Ctcp);

    server_line(&mut server, b":PAPO-DRIVER!p@localhost PRIVMSG #papo :sent\r\nBATCH +history42 chathistory #papo\r\n@batch=history42 :dan!d@localhost PRIVMSG #papo :earlier\r\n").await;
    assert_eq!(next_message(&mut events).await.origin(), Origin::Echo);
    assert_eq!(next_message(&mut events).await.origin(), Origin::Replay);
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
async fn registration_is_forwarded() {
    let (mut server, client, mut events, driver) = scripted(config());
    assert!(!client.state().is_connected());
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
    assert!(client.state().is_connected());
    within(server.write_all(b":ergo.test CAP * LS :\r\n"))
        .await
        .unwrap();
    let mut cap_end = [0; 9];
    within(server.read_exact(&mut cap_end)).await.unwrap();
    assert_eq!(&cap_end, b"CAP END\r\n");
    within(server.write_all(b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n"))
        .await
        .unwrap();
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
    assert!(!client.state().is_connected());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn connection_failures_retry_up_to_the_configured_limit() {
    let calls = Arc::new(AtomicUsize::new(0));
    let connector: super::Connector = Arc::new({
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(unavailable("scripted connection failure")) })
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
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
        Box::new(|| 0),
    );
    let driver = tokio::spawn(driver);
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            retry_in: Some(retry_in),
            ..
        })) if retry_in == Duration::from_millis(800)
    ));
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if !disconnected.is_retrying()
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Connect(_))
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn registration_waiter_fails_after_one_connection_attempt() {
    let calls = Arc::new(AtomicUsize::new(0));
    let connector: super::Connector = Arc::new({
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(unavailable("scripted connection failure")) })
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
        super::PostRegistration::default(),
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
async fn registration_disconnect_does_not_enter_reconnect_backoff() {
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
        super::PostRegistration::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
    within(server.shutdown()).await.unwrap();

    assert!(matches!(
        within(waiting).await.unwrap(),
        Err(Error::Registration(_))
    ));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if !disconnected.is_retrying()
    ));
    assert!(within(driver).await.unwrap().is_err());
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
        super::PostRegistration::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);

    complete_registration(&mut first_server, &mut events).await;
    let join = tokio::spawn({
        let client = client.clone();
        async move { client.join("#papo").key("persistent-channel-key").await }
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
    registration_burst(&mut second_server).await;
    server_line(&mut second_server, b":ergo.test CAP * LS :\r\n").await;
    client_line(&mut second_server, b"CAP END\r\n").await;
    server_line(
        &mut second_server,
        b":ergo.test 001 papo-driver :Welcome back\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
    )
    .await;
    assert!(
        matches!(events.next().await, Some(Event::Registered(registered)) if registered.reconnected)
    );
    client_line(&mut second_server, b"JOIN #papo persistent-channel-key\r\n").await;
    server_line(
        &mut second_server,
        b":ergo.test 475 papo-driver #papo :Cannot join channel (+k)\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Raw(_))));
    within(second_server.shutdown()).await.unwrap();
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));

    tokio::time::advance(Duration::from_secs(3)).await;
    registration_burst(&mut third_server).await;
    server_line(&mut third_server, b":ergo.test CAP * LS :\r\n").await;
    client_line(&mut third_server, b"CAP END\r\n").await;
    server_line(&mut third_server, b":ergo.test 001 papo-driver :Welcome back\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), third_server.read_u8())
            .await
            .is_err()
    );
    assert!(client.state().is_connected());

    let quit = tokio::spawn(async move { client.quit(None).await });
    client_line(&mut third_server, b"QUIT\r\n").await;
    within(third_server.shutdown()).await.unwrap();
    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
}

#[test]
fn rejoin_set_tracks_our_joins_parts_and_kicks() {
    let session = Session::new(config());
    let mut defaults = super::PostRegistration {
        bot: false,
        channels: vec![super::Autojoin::new(
            "#staff",
            Some("private-channel-key".into()),
        )],
        history_on_reconnect: true,
    };

    super::update_rejoin_channels(
        &session,
        &mut defaults,
        &Event::Joined(crate::event::Joined {
            channel: "#papo".into(),
            nick: "papo-driver".into(),
            is_self: true,
        }),
    );
    super::update_rejoin_channels(
        &session,
        &mut defaults,
        &Event::Joined(crate::event::Joined {
            channel: "#visitors".into(),
            nick: "alice".into(),
            is_self: false,
        }),
    );
    assert_eq!(defaults.channels.len(), 2);

    super::update_rejoin_channels(
        &session,
        &mut defaults,
        &Event::Parted(crate::event::Parted {
            channel: "#Papo".into(),
            nick: "papo-driver".into(),
            reason: None,
            is_self: true,
        }),
    );
    super::update_rejoin_channels(
        &session,
        &mut defaults,
        &Event::Kicked(crate::event::Kicked {
            channel: "#staff".into(),
            nick: "papo-driver".into(),
            by: crate::Source::User {
                nick: "chanserv".into(),
                user: None,
                host: None,
            },
            reason: Some("access removed".into()),
            is_self: true,
        }),
    );
    assert!(defaults.channels.is_empty());
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
        super::PostRegistration::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
        Box::new(|| 0),
    );
    let driver = tokio::spawn(driver);
    complete_registration(&mut first_server, &mut events).await;

    let reconnect_requested_at = tokio::time::Instant::now();
    within(client.reconnect()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            retry_in: Some(Duration::ZERO),
            ..
        }))
    ));
    registration_burst(&mut second_server).await;
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
        matches!(events.next().await, Some(Event::Registered(registered)) if registered.reconnected)
    );

    let quit = tokio::spawn(async move { client.quit(None).await });
    client_line(&mut second_server, b"QUIT\r\n").await;
    within(second_server.shutdown()).await.unwrap();
    within(quit).await.unwrap().unwrap();
    within(driver).await.unwrap().unwrap();
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reconnect_history_pages_deduplicate_into_one_non_live_event() {
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
        super::PostRegistration {
            bot: false,
            channels: vec!["#papo".into()],
            history_on_reconnect: true,
        },
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);

    complete_history_registration(&mut first_server, &mut events).await;
    client_line(&mut first_server, b"JOIN #papo\r\n").await;
    server_line(&mut first_server, b"@time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120000 :emily!~e@localhost PRIVMSG #papo :before disconnect\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    within(first_server.shutdown()).await.unwrap();
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));
    tokio::time::advance(Duration::from_secs(2)).await;

    registration_burst(&mut second_server).await;
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
        matches!(events.next().await, Some(Event::Registered(registered)) if registered.reconnected)
    );
    client_line(&mut second_server, b"JOIN #papo\r\n").await;
    client_line(
        &mut second_server,
        b"@label=0 CHATHISTORY AFTER #papo msgid=RfcBTzyxY120000 100\r\n",
    )
    .await;
    server_line(&mut second_server, b"@label=0 :ergo.test BATCH +reply0 labeled-response\r\n@batch=reply0 :ergo.test BATCH +history0 chathistory #papo\r\n@batch=history0;time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120000 :emily!~e@localhost PRIVMSG #papo :before disconnect\r\n@batch=history0;time=2026-10-15T12:01:00.000Z;msgid=RfcBTzyxY120001 :emily!~e@localhost PRIVMSG #papo :missed one\r\n@batch=reply0 :ergo.test BATCH -history0\r\n:ergo.test BATCH -reply0\r\n").await;
    client_line(
        &mut second_server,
        b"@label=1 CHATHISTORY AFTER #papo msgid=RfcBTzyxY120001 100\r\n",
    )
    .await;
    server_line(&mut second_server, b"@label=1 :ergo.test BATCH +reply1 labeled-response\r\n@batch=reply1;draft/chathistory-end :ergo.test BATCH +history1 chathistory #papo\r\n@batch=history1;time=2026-10-15T12:01:00.000Z;msgid=RfcBTzyxY120001 :emily!~e@localhost PRIVMSG #papo :missed one\r\n@batch=history1;time=2026-10-15T12:02:00.000Z;msgid=RfcBTzyxY120002 :emily!~e@localhost PRIVMSG #papo :missed two\r\n@batch=reply1 :ergo.test BATCH -history1\r\n:ergo.test BATCH -reply1\r\n").await;

    let Some(Event::History(history)) = events.next().await else {
        panic!("expected aggregate history event")
    };
    assert_eq!(history.target, "#papo");
    assert_eq!(
        history
            .messages()
            .map(ChatMessage::text)
            .collect::<Vec<_>>(),
        ["missed one", "missed two"]
    );
    assert!(
        history
            .messages()
            .all(|message| message.origin() == Origin::History && !message.is_live())
    );
    assert!(history.end);
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
                papo_core::HistoryItem::Message(papo_core::HistoryMessage {
                    message: Message::privmsg("#papo", format!("backlog message {index}"))
                        .with_tag("msgid", format!("RfcBTzyxY12{index:04}")),
                    context: false,
                })
            }),
        );
        assert_eq!(catchup.items.len(), 1_000);
        assert_eq!(next.as_deref(), Some("RfcBTzyxY120999"));
    }
}

#[test]
fn persistence_gates_rejoin_and_history_fetch() {
    for (effective, expected) in [(false, true), (true, false)] {
        let now = std::time::Instant::now();
        let mut session = Session::new(config().wanted_caps(["draft/persistence"]));
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
        Err(unavailable("scripted connection failure")),
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
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if disconnected.is_retrying()
    ));

    assert!(matches!(
        within(client.send_raw("WHOIS emily")).await,
        Err(disconnected!())
    ));
    assert!(matches!(
        within(client.privmsg("#papo", "lost during reconnect")).await,
        Err(disconnected!())
    ));
    assert!(matches!(
        within(client.whois("emily")).await,
        Err(disconnected!())
    ));

    tokio::time::advance(Duration::from_secs(2)).await;
    registration_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
    client_line(&mut server, b"CAP END\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
    let quit = tokio::spawn(async move { client.quit(None).await });
    client_line(&mut server, b"QUIT\r\n").await;
    within(server.shutdown()).await.unwrap();
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
            Box::pin(async { Err(unavailable("scripted connection failure")) })
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
    within(client.quit(Some("cancel reconnect"))).await.unwrap();
    assert!(within(driver).await.unwrap().is_ok());
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
        super::PostRegistration::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    complete_registration(&mut server, &mut events).await;
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
    within(client.quit(None)).await.unwrap();
    assert!(within(driver).await.unwrap().is_ok());
}

fn failing_driver_with_one_event_slot() -> (super::Client, super::Events, Driver, Arc<AtomicUsize>)
{
    let calls = Arc::new(AtomicUsize::new(0));
    let connector: super::Connector = Arc::new({
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(unavailable("scripted connection failure")) })
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
        super::PostRegistration::default(),
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

    within(client.quit(None)).await.unwrap();
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
            Box::pin(async { Err(unavailable("scripted connection failure")) })
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
        Err(unavailable("first scripted failure")),
        Err(unavailable("second scripted failure")),
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
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
        Box::new(|| 0),
    );
    let driver = tokio::spawn(driver);
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));

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
    within(stable_server.shutdown()).await.unwrap();
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));

    registration_burst(&mut next_server).await;
    let delay = {
        let attempts = attempts.lock().unwrap();
        assert_eq!(attempts.len(), 4);
        attempts[3] - disconnected_at
    };
    assert_eq!(delay, Duration::from_millis(800));
    let quit = tokio::spawn(async move { client.quit(None).await });
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
        super::PostRegistration::default(),
        Reconnect::exponential(),
        Box::new(store),
        connector,
    );
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
    let quit = tokio::spawn(async move { client.quit(None).await });
    client_line(&mut server, b"QUIT\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn connector_failure_under_stored_sts_is_terminal() {
    let calls = Arc::new(AtomicUsize::new(0));
    let connector: super::Connector = Arc::new({
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(unavailable("scripted TLS failure")) })
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
        Box::new(store),
        connector,
    );

    assert!(matches!(within(driver).await, Err(Error::Connect(_))));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if !disconnected.is_retrying()
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn eof_emits_disconnected_and_returns_an_error() {
    let (mut server, _client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::ConnectionLost,
            retry_in: None,
        }))
    ));
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_error_overrides_a_non_io_terminal_cause() {
    let (mut server, _client, mut events, driver) = scripted_with_failures(false);
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Protocol { message },
            retry_in: None,
        })) if message == "shutdown failed"
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Protocol { message },
            ..
        })) if message == "shutdown failed"
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_error_does_not_replace_an_io_terminal_cause() {
    let (mut server, _client, mut events, driver) = scripted_with_failures(true);
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Protocol { message },
            retry_in: None,
        })) if message == "original read failure"
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Protocol { message },
            ..
        })) if message == "original read failure"
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn overlong_line_becomes_a_terminal_disconnect() {
    let (mut server, _client, mut events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
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
    registration_burst(&mut server).await;
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
    registration_burst(&mut server).await;
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
async fn required_sasl_failure_rejects_registration_waiter() {
    let settings = config()
        .wanted_caps(["sasl"])
        .sasl(Sasl::plain("alice", "correct horse battery staple"));
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

    assert!(matches!(
        within(waiter).await.unwrap(),
        Err(Error::Registration(crate::RegistrationError::Sasl { message }))
            if message.contains("SASL")
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
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
        super::PostRegistration::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :sasl=PLAIN\r\n").await;
    client_line(&mut server, b"CAP REQ :sasl\r\n").await;
    server_line(&mut server, b":ergo.test CAP * NAK :sasl\r\n").await;

    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(disconnected)) if !disconnected.is_retrying()
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
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

    assert!(within(waiter).await.unwrap().is_ok());
    assert!(matches!(
        events.next().await,
        Some(Event::Message(message)) if message.kind() == MessageKind::Notice
    ));
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
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
        .register_on_connect(
            Some("alice"),
            Some("alice@example.org"),
            "correct horse battery staple",
        );
    let (_client, mut events, driver) = Driver::with_stream_and_registration(
        client_stream,
        Session::new(config),
        super::DEFAULT_CHANNEL_CAPACITY,
        Some(registration),
        super::PostRegistration::default(),
    );
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
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
        Some(Event::AccountRegistration(crate::AccountRegistration::VerificationRequired { account, message }))
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
    let (_client, mut events, driver) = Driver::with_stream_and_registration(
        client_stream,
        Session::new(config()),
        super::DEFAULT_CHANNEL_CAPACITY,
        None,
        super::PostRegistration {
            bot: true,
            channels: vec![
                "#papo".into(),
                super::Autojoin::new("#PAPO", Some("replacement-channel-key".into())),
                super::Autojoin::new("#papo-bots", Some("private-channel-key".into())),
            ],
            history_on_reconnect: false,
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
        b"MODE papo-driver +B\r\nJOIN #PAPO replacement-channel-key\r\nJOIN #papo-bots private-channel-key\r\n",
    )
    .await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn bot_default_without_advertised_mode_sends_no_mode() {
    assert_registration_output(
        super::PostRegistration {
            bot: true,
            channels: Vec::new(),
            history_on_reconnect: false,
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
            history_on_reconnect: false,
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
    registration_burst(&mut server).await;
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
        super::PostRegistration::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    registration_burst(&mut first_server).await;
    server_line(&mut first_server, b":ergo.test CAP * LS :sts=port=6699\r\n").await;
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            retry_in: Some(Duration::ZERO),
            ..
        }))
    ));
    registration_burst(&mut second_server).await;
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

    let quit = tokio::spawn(async move { client.quit(None).await });
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
        super::PostRegistration::default(),
        Reconnect::exponential().max_attempts(1),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
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
        Err(Error::Registration(crate::RegistrationError::StsUpgradeLoop { host }))
            if host == "irc.example.net"
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
    registration_burst(&mut server).await;
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
        super::PostRegistration::default(),
        Reconnect::never(),
        Box::new(store),
        connector,
    );
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
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
        super::PostRegistration::default(),
        Reconnect::never(),
        Box::new(store),
        connector,
    );
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
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
        super::PostRegistration::default(),
        Reconnect::never(),
        Box::new(store),
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
    client_line(&mut server, b"CAP END\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn client_send_raw_parses_and_obeys_the_rate_limit() {
    let (mut server, client, _events, driver) =
        scripted(config().rate_limit(papo_core::RateLimit::new(1, Duration::from_millis(500))));
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
    client.send_raw("WHOIS emily").await.unwrap();
    client.send_raw("WHOIS alice").await.unwrap();
    client_line(&mut server, b"WHOIS emily\r\n").await;
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );
    tokio::time::advance(Duration::from_millis(500)).await;
    client_line(&mut server, b"WHOIS alice\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
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
    let whois = within(whois).await.unwrap().unwrap();
    assert_eq!(
        (whois.nick.as_ref(), whois.user.as_deref()),
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
    assert_eq!(whois.away.as_deref(), Some("away from keyboard"));
    assert!(whois.is_bot && whois.secure);
    assert_eq!(whois.idle, Some(Duration::from_secs(42)));

    server_line(
        &mut server,
        b":ergo.test 005 papo-driver WHOX PREFIX=(ov)@+ CHANMODES=beI,k,l,imnst :are supported\r\n",
    )
    .await;
    let who = tokio::spawn({
        let client = client.clone();
        async move { client.who("#papo").await }
    });
    client_line(&mut server, b"@label=1 WHO #papo %tcuhnfar\r\n").await;
    server_line(&mut server, b"@label=1 :ergo.test BATCH +who-42 labeled-response\r\n@batch=who-42 :ergo.test 354 papo-driver 0 #papo ~alice client.example alice H@B alice :Alice Example\r\n@batch=who-42 :ergo.test 354 papo-driver 0 #papo ~dan user.example dan G+* 0 :Dan Away\r\n@batch=who-42 :ergo.test 315 papo-driver #papo :End of WHO list\r\n:ergo.test BATCH -who-42\r\n").await;
    let who = within(who).await.unwrap().unwrap();
    assert_eq!(who.len(), 2);
    assert_eq!(
        (who[0].nick.as_ref(), who[0].account.as_deref()),
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
    client_line(&mut server, b"@label=8 WHOIS missing-nick\r\n").await;
    server_line(
        &mut server,
        b"@label=8 :ergo.test 401 papo-driver missing-nick :No such nick\r\n",
    )
    .await;
    assert!(matches!(
        within(request).await.unwrap(),
        Err(Error::Server(error)) if error.code() == "401"
    ));
    let timed_out = tokio::spawn({
        let client = client.clone();
        async move { client.request_raw("WHOIS alice").await }
    });
    client_line(&mut server, b"@label=9 WHOIS alice\r\n").await;
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        within(timed_out).await.unwrap(),
        Err(Error::Timeout)
    ));

    let aborted = tokio::spawn(async move { client.request_raw("WHOIS emily").await });
    client_line(&mut server, b"@label=A WHOIS emily\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(aborted).await.unwrap(),
        Err(disconnected!())
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn request_raw_returns_state_changing_lines_without_an_event() {
    let (mut server, client, mut events, driver) =
        scripted(config().wanted_caps(["labeled-response"]));
    let driver = tokio::spawn(driver);
    complete_labelled_registration(&mut server, &mut events).await;

    let renamed = tokio::spawn({
        let client = client.clone();
        async move { client.request_raw("NICK papo-renamed").await }
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
async fn mode_helpers_await_every_command_and_return_refusals() {
    let (mut server, client, mut events, driver) =
        scripted(config().wanted_caps(["labeled-response"]));
    let driver = tokio::spawn(driver);
    complete_labelled_registration(&mut server, &mut events).await;
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
    within(modes).await.unwrap().unwrap();

    let refused = tokio::spawn({
        let client = client.clone();
        async move { client.mode("#papo").deop("alice").await }
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
#[allow(clippy::too_many_lines)]
async fn typed_requests_match_unlabelled_responses() {
    let (mut server, client, mut events, driver) =
        scripted(config().rate_limit(papo_core::RateLimit::unlimited()));
    let driver = tokio::spawn(driver);
    complete_registration(&mut server, &mut events).await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver PREFIX=(ov)@+ CHANMODES=beI,k,l,imnst MODES=3 :are supported\r\n",
    )
    .await;

    let whois = tokio::spawn({
        let client = client.clone();
        async move { client.whois("alice").await }
    });
    client_line(&mut server, b"WHOIS alice\r\n").await;
    server_line(&mut server, b":ergo.test 311 papo-driver alice ~alice host * :Alice Example\r\n:ergo.test 318 papo-driver alice :End of /WHOIS list\r\n").await;
    assert_eq!(within(whois).await.unwrap().unwrap().nick, "alice");

    let who = tokio::spawn({
        let client = client.clone();
        async move { client.who("#papo").await }
    });
    client_line(&mut server, b"WHO #papo\r\n").await;
    server_line(&mut server, b":ergo.test 352 papo-driver #papo ~alice host ergo.test alice H@ :0 Alice Example\r\n:ergo.test 315 papo-driver #papo :End of WHO list\r\n").await;
    assert_eq!(within(who).await.unwrap().unwrap()[0].nick, "alice");

    let join = tokio::spawn({
        let client = client.clone();
        async move { client.join("#papo").await }
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
        async move { client.topic("#papo", None).await }
    });
    client_line(&mut server, b"TOPIC #papo\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 332 papo-driver #papo :Ergonomic IRC\r\n",
    )
    .await;
    assert_eq!(
        within(topic).await.unwrap().unwrap().as_deref(),
        Some("Ergonomic IRC")
    );

    let part = tokio::spawn({
        let client = client.clone();
        async move { client.part("#papo", "").await }
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
        async move { client.join("#invite-only").await }
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
        client.request_raw("VERSION").await,
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
async fn capability_loss_fails_a_pending_request_immediately() {
    let (mut server, client, mut events, driver) =
        scripted(config().wanted_caps(["labeled-response"]));
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :labeled-response\r\n").await;
    client_line(&mut server, b"CAP REQ :labeled-response\r\n").await;
    server_line(&mut server, b":ergo.test CAP * ACK :labeled-response\r\n").await;
    client_line(&mut server, b"CAP END\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :No MOTD\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Registered(_))));

    let request = tokio::spawn(async move { client.request_raw("VERSION").await });
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
    complete_registration(&mut server, &mut events).await;
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
    registration_burst(&mut server).await;
    let join = client
        .join("#ergo-ops")
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

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn labelled_join_timeout_kick_and_part_end_the_action() {
    let settings = config()
        .wanted_caps(["labeled-response"])
        .request_timeout(Duration::from_secs(1));
    let (mut server, client, mut events, driver) = scripted(settings);
    let driver = tokio::spawn(driver);
    complete_labelled_registration(&mut server, &mut events).await;

    let stalled = tokio::spawn({
        let client = client.clone();
        async move { client.join("#stalled").await }
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
        async move { client.join("#limited").await }
    });
    client_line(&mut server, b"@label=1 JOIN #limited\r\n").await;
    server_line(&mut server, b"@label=1 :papo-driver!papo@localhost JOIN #limited * :papo driver test\r\n:ergo.test KICK #limited papo-driver :Channel is full\r\n").await;
    assert!(matches!(
        within(kicked).await.unwrap(),
        Err(disconnected!())
    ));

    let parted = tokio::spawn({
        let client = client.clone();
        async move { client.join("#registered").await }
    });
    client_line(&mut server, b"@label=2 JOIN #registered\r\n").await;
    server_line(&mut server, b"@label=2 :papo-driver!papo@localhost JOIN #registered * :papo driver test\r\n:papo-driver!papo@localhost PART #registered :Registration required\r\n").await;
    assert!(matches!(
        within(parted).await.unwrap(),
        Err(disconnected!())
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
}

#[tokio::test(flavor = "current_thread")]
async fn chat_text_uses_plain_splitting_and_rejects_empty_input() {
    let (mut server, client, _events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
    assert_eq!(
        within(client.privmsg("#papo", "a".repeat(600)))
            .await
            .unwrap(),
        super::Sent::without_echo("#papo")
    );
    let lines = format!(
        "PRIVMSG #papo {}\r\nPRIVMSG #papo {}\r\n",
        "a".repeat(413),
        "a".repeat(187)
    );
    client_line(&mut server, lines.as_bytes()).await;
    client
        .notice("papo-helper", "maintenance soon")
        .await
        .unwrap();
    client_line(&mut server, b"NOTICE papo-helper :maintenance soon\r\n").await;
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
    registration_burst(&mut server).await;
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
    complete_caps_registration(
        &mut server,
        &mut events,
        "echo-message labeled-response message-tags",
        "echo-message labeled-response message-tags",
    )
    .await;

    let sender = client.clone();
    let sent = tokio::spawn(async move {
        sender
            .privmsg("papo-helper", "fixed in 69876ec")
            .reply_to(&crate::MessageId::from("RfcBTzyxY120004"))
            .channel_context(&crate::ChannelName::from("#papo"))
            .tag("+example.org/trace", "98")
            .await
    });
    client_line(
        &mut server,
        b"@+example.org/trace=98;+reply=RfcBTzyxY120004;+channel-context=#papo;label=0 PRIVMSG papo-helper :fixed in 69876ec\r\n",
    )
    .await;
    server_line(
        &mut server,
        b"@label=0;msgid=RfcBTzyxY120005 :papo-driver!papo@localhost PRIVMSG papo-helper :fixed in 69876ec\r\n",
    )
    .await;
    assert_eq!(
        within(sent).await.unwrap().unwrap().id.unwrap(),
        "RfcBTzyxY120005"
    );
    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn chat_text_returns_metadata_from_a_labelled_echo() {
    let config = config().wanted_caps(["echo-message", "labeled-response"]);
    let (mut server, client, _events, driver) = scripted(config);
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
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
    assert_eq!(sent.id.as_deref(), Some("RfcBTzyxY120004"));
    assert_eq!(Some(sent.target.as_ref()), Some("#papo"));
    assert_eq!(
        sent.time,
        Some(UNIX_EPOCH + Duration::new(1_770_091_506, 789_000_000))
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn chat_text_returns_a_labelled_numeric_refusal() {
    let settings = config().wanted_caps(["labeled-response"]);
    let (mut server, client, mut events, driver) = scripted(settings);
    let task = tokio::spawn(driver);
    complete_labelled_registration(&mut server, &mut events).await;

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
        Err(Error::Server(error)) if error.kind() == crate::Refusal::CannotSendToChannel
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn reactions_obey_clienttagdeny() {
    assert_reaction_wire(
        "",
        false,
        Some(b"@+reply=RfcBTzyxY120004;+draft/react=\xF0\x9F\x91\x8D TAGMSG #papo\r\n"),
    )
    .await;
    assert_reaction_wire(
        "",
        true,
        Some(b"@+reply=RfcBTzyxY120004;+draft/unreact=\xF0\x9F\x91\x8D TAGMSG #papo\r\n"),
    )
    .await;
    assert_reaction_wire(
        ":ergo.test 005 papo-driver CLIENTTAGDENY=draft/react :are supported\r\n",
        false,
        None,
    )
    .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn typing_throttles_active_per_target_and_sends_terminal_states() {
    let (mut server, client, _events, driver) = scripted(config().wanted_caps(["message-tags"]));
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
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

#[tokio::test(flavor = "current_thread")]
async fn redaction_requires_its_capability_and_propagates_fail() {
    let caps = ["draft/message-redaction", "labeled-response"];
    let (mut server, client, _events, driver) = scripted(config().wanted_caps(caps));
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
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
                &crate::Target::from("#papo"),
                &crate::MessageId::from("RfcBTzyxY120004"),
                Some("duplicate message"),
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
            .redact(
                &crate::Target::from("#papo"),
                &crate::MessageId::from("RfcBTzyxY120005"),
                None,
            )
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
        Err(Error::Server(error))
            if error.command() == "REDACT" && error.code() == "FORBIDDEN"
    ));

    server_line(
        &mut server,
        b":ergo.test CAP * DEL :draft/message-redaction\r\n",
    )
    .await;
    while client.state().has_cap("draft/message-redaction") {
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        within(client.redact(
            &crate::Target::from("#papo"),
            &crate::MessageId::from("RfcBTzyxY120006"),
            None,
        ))
        .await,
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
async fn labels_distinguish_our_echo_from_another_session() {
    let caps = ["echo-message", "labeled-response"];
    let (mut server, client, mut events, driver) = scripted(config().wanted_caps(caps));
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
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
    while !matches!(events.next().await, Some(Event::Registered(_))) {}

    let sender = client.clone();
    let sent = tokio::spawn(async move { sender.privmsg("#papo", "from this device").await });
    client_line(&mut server, b"@label=0 PRIVMSG #papo :from this device\r\n").await;
    server_line(
        &mut server,
        b"@label=0;msgid=RfcBTzyxY120004 :papo-driver!papo@localhost PRIVMSG #papo :from this device\r\n",
    )
    .await;
    assert!(within(sent).await.unwrap().is_ok());
    let echo = next_message(&mut events).await;
    assert_eq!(echo.origin(), Origin::Echo);
    assert!(!echo.is_live());

    server_line(
        &mut server,
        b"@msgid=RfcBTzyxY120005 :papo-driver!papo@localhost PRIVMSG #papo :from another device\r\n",
    )
    .await;
    let other = next_message(&mut events).await;
    assert_eq!(other.origin(), Origin::OtherSession);
    assert!(!other.is_live());

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
async fn client_send_raw_rejects_malformed_or_delimited_input() {
    let (mut server, client, _events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
    for line in [
        "",
        "   ",
        "WHOIS emily\r\n",
        "WHOIS emily\nPRIVMSG #papo :injected",
        "PING \0token",
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
async fn quit_resolves_after_server_eof() {
    let (mut server, client, mut events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
    let quit = tokio::spawn(async move { client.quit(Some("leaving for maintenance")).await });
    let mut line = [0; 31];
    within(server.read_exact(&mut line)).await.unwrap();
    assert_eq!(&line, b"QUIT :leaving for maintenance\r\n");
    within(server.shutdown()).await.unwrap();
    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
    assert!(matches!(events.next().await, Some(Event::Disconnected(_))));
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
    within(server.write_all(b":ergo.test NOTICE papo-driver :connection closing\r\n"))
        .await
        .unwrap();

    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Requested,
            retry_in: None,
        }))
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn quit_times_out_after_ten_seconds() {
    let (mut server, client, mut events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
    let quit = tokio::spawn(async move { client.quit(None).await });
    let mut line = [0; 6];
    within(server.read_exact(&mut line)).await.unwrap();
    assert_eq!(&line, b"QUIT\r\n");
    tokio::time::advance(Duration::from_secs(9)).await;
    assert!(!quit.is_finished());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(within(quit).await.unwrap(), Err(Error::Timeout)));
    assert!(within(driver).await.unwrap().is_ok());
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected(crate::Disconnected {
            cause: crate::DisconnectCause::Requested,
            retry_in: None,
        }))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_every_client_sends_quit() {
    let (mut server, client, _events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
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
    registration_burst(&mut server).await;
    within(server.write_all(b":ergo.test NOTICE papo-driver :maintenance soon\r\n:ergo.test NOTICE papo-driver :still queued\r\n"))
        .await
        .unwrap();
    while events.receiver.len() != 1 {
        tokio::task::yield_now().await;
    }

    let quit = tokio::spawn(async move { client.quit(None).await });
    let mut line = [0; 6];
    within(server.read_exact(&mut line)).await.unwrap();
    assert_eq!(&line, b"QUIT\r\n");
    tokio::time::advance(Duration::from_secs(10)).await;

    assert!(matches!(within(quit).await.unwrap(), Err(Error::Timeout)));
    assert!(within(driver).await.unwrap().is_ok());
    assert_eq!(events.receiver.len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn only_pending_request_enables_read_ahead() {
    let (mut server, client, mut events, driver) = scripted_with_capacity(config(), 2);
    let driver = tokio::spawn(driver);
    complete_registration(&mut server, &mut events).await;
    saturate_events(&mut server, &events).await;

    let whois = tokio::spawn(async move { client.whois("alice").await });
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

#[tokio::test(flavor = "current_thread")]
async fn only_channel_sync_waiter_enables_read_ahead() {
    let (mut server, client, mut events, driver) = scripted_with_capacity(config(), 2);
    let driver = tokio::spawn(driver);
    complete_registration(&mut server, &mut events).await;
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
async fn only_read_marker_waiter_enables_read_ahead() {
    let settings = config().wanted_caps(["draft/read-marker", "labeled-response"]);
    let (mut server, client, mut events, driver) = scripted_with_capacity(settings, 2);
    let driver = tokio::spawn(driver);
    complete_read_marker_registration(&mut server, &mut events).await;
    saturate_events(&mut server, &events).await;

    let marker = tokio::spawn(async move { client.read_marker("#papo").await });
    client_line(&mut server, b"MARKREAD #papo\r\n").await;
    server_line(
        &mut server,
        b":ergo.test MARKREAD #papo timestamp=2025-10-15T11:55:42.123Z\r\n",
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
    complete_registration(&mut server, &mut events).await;
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
    complete_registration(&mut server, &mut events).await;
    let whois = tokio::spawn(async move { client.whois("alice").await });
    client_line(&mut server, b"WHOIS alice\r\n").await;
    server_line(
        &mut server,
        b":ergo.test NOTICE papo-driver :maintenance soon\r\n:ergo.test NOTICE papo-driver :maintenance extended\r\nERROR :Closing Link: papo-driver (Server shutdown)\r\n",
    )
    .await;

    assert!(matches!(within(whois).await.unwrap(), Err(disconnected!())));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(disconnected!())
    ));
    assert_eq!(next_message(&mut events).await.text(), "maintenance soon");
    assert_eq!(
        next_message(&mut events).await.text(),
        "maintenance extended"
    );
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
    complete_registration(&mut server, &mut events).await;
    let whois = tokio::spawn(async move { client.whois("alice").await });
    client_line(&mut server, b"WHOIS alice\r\n").await;
    server_line(
        &mut server,
        b":ergo.test NOTICE papo-driver :maintenance one\r\n:ergo.test NOTICE papo-driver :maintenance two\r\n:ergo.test NOTICE papo-driver :maintenance three\r\n",
    )
    .await;

    assert!(matches!(within(whois).await.unwrap(), Err(disconnected!())));
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
async fn session_deadline_sends_keepalive_ping() {
    let config = config()
        .ping_interval(Duration::from_secs(30))
        .ping_timeout(Duration::from_secs(120));
    let (mut server, _client, _events, driver) = scripted(config);
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
    tokio::time::advance(Duration::from_secs(30)).await;
    let mut ping = [0; 14];
    within(server.read_exact(&mut ping)).await.unwrap();
    assert_eq!(&ping, b"PING :papo-0\r\n");
    within(server.shutdown()).await.unwrap();
    assert!(matches!(within(task).await.unwrap(), Err(disconnected!())));
}

#[tokio::test(flavor = "current_thread")]
async fn client_reads_owned_state_snapshots() {
    let (mut server, client, mut events, driver) = scripted(config());
    assert!(!client.state().is_connected());
    let driver = tokio::spawn(driver);
    complete_registration(&mut server, &mut events).await;
    assert!(client.state().is_connected());

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
    assert!(!client.state().away());

    server_line(&mut server, b":ergo.test 005 papo-driver CASEMAPPING=ascii CHANTYPES=# PREFIX=(ov)@+ MONITOR=100 NICKLEN=27 :are supported\r\n:ergo.test 306 papo-driver :You have been marked as being away\r\n:papo-driver!papo@localhost MODE papo-driver +iw\r\n:papo-driver!papo@localhost JOIN #Papo\r\n:ergo.test 353 papo-driver = #Papo :@papo-driver alice!~alice@cloak.example\r\n:ergo.test 366 papo-driver #Papo :End of NAMES list\r\n:ergo.test 730 papo-driver :bob!bob@users.example\r\n").await;
    while !matches!(events.next().await, Some(Event::MonitorOnline(_))) {}

    let nick: crate::Nick = client.nick();
    assert_eq!(nick, "papo-driver");
    let state = client.state();
    assert!(state.is_me(&nick));
    assert_eq!(state.account().map(AsRef::as_ref), Some("emily"));
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
    assert_eq!(
        state.user(crate::Nick::from("ALICE")).unwrap().username(),
        Some("~alice")
    );
    assert_eq!(state.monitor_online().next().unwrap().nick(), "bob");
    assert_eq!(state.isupport().nicklen(), Some(27));
    assert!(
        state
            .features()
            .any(|feature| feature == papo_core::Feature::Monitor)
    );
    let snapshot = client.state();
    assert_eq!(snapshot.nick(), "papo-driver");
    assert_eq!(snapshot.account().map(AsRef::as_ref), Some("emily"));

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
    complete_extended_monitor_registration(&mut server, &mut events).await;
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
        &papo_core::Account::LoggedIn("emily".into())
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
        Some("Gone to lunch")
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

    let state = client.state();
    assert!(state.has_cap("no-implicit-names"));
    assert!(state.has_cap("draft/no-implicit-names"));
    assert!(!state.has_cap("message-tags"));
    assert_eq!(state.isupport().nicklen(), Some(27));
    assert!(state.is_connected());
    assert!(state.is_me("PAPO-DRIVER"));
    assert!(state.casemapping().eq("#Papo", "#papo"));
    assert!(!state.casemapping().eq("#PAPO[dev]", "#papo{dev}"));
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
    assert!(matches!(events.next().await, Some(Event::MonitorOnline(_))));
    assert!(matches!(events.next().await, Some(Event::MonitorOnline(_))));

    server_line(&mut server, b":ergo.test 731 papo-driver :alice\r\n").await;
    assert!(
        matches!(events.next().await, Some(Event::MonitorOffline(offline)) if client.state().casemapping().eq(&offline.nick, "alice"))
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
    complete_extended_monitor_registration(&mut server, &mut events).await;
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
