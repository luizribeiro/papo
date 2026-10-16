use super::test_support::*;
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

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(task).await.unwrap(),
        Err(Error::Disconnected)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn registration_is_forwarded() {
    let (mut server, client, mut events, driver) = scripted(config());
    assert!(!client.is_connected());
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
    assert!(client.is_connected());
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
    assert!(matches!(
        within(task).await.unwrap(),
        Err(Error::Disconnected)
    ));
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
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Unsupported(_))
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
            Box::pin(async { Err(Error::Unsupported("scripted connection failure")) })
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
        256,
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);

    assert!(matches!(
        within(waiting).await.unwrap(),
        Err(Error::Unsupported("scripted connection failure"))
    ));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected {
            will_retry: false,
            ..
        })
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
        256,
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
        Some(Event::Disconnected {
            will_retry: false,
            ..
        })
    ));
    assert!(within(driver).await.unwrap().is_err());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
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
        Session::new(config().wanted_caps([
            "draft/chathistory",
            "labeled-response",
            "batch",
            "server-time",
        ])),
        super::DEFAULT_CHANNEL_CAPACITY,
        None,
        super::PostRegistration {
            bot: false,
            channels: vec!["#papo".into()],
            history_on_reconnect: false,
        },
        Reconnect::exponential(),
        256,
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);

    complete_history_registration(&mut first_server, &mut events).await;
    client_line(&mut first_server, b"JOIN #papo\r\n").await;
    server_line(&mut first_server, b"@time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120000 :emily!~e@localhost PRIVMSG #papo :before disconnect\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    assert!(client.is_connected());
    within(first_server.shutdown()).await.unwrap();
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
    server_line(
        &mut second_server,
        b":ergo.test 005 papo-driver CHATHISTORY=100 MSGREFTYPES=msgid,timestamp :supported\r\n:ergo.test 001 papo-driver :Welcome back\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Reconnected(_))));
    client_line(&mut second_server, b"JOIN #papo\r\n").await;
    assert!(
        tokio::time::timeout(Duration::from_millis(1), second_server.read_u8())
            .await
            .is_err()
    );
    assert!(client.is_connected());

    let quit = tokio::spawn(async move { client.quit(None).await });
    client_line(&mut second_server, b"QUIT\r\n").await;
    within(second_server.shutdown()).await.unwrap();
    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
}

#[tokio::test(flavor = "current_thread")]
async fn explicit_reconnect_redials_without_backoff() {
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
        super::PostRegistration::default(),
        Reconnect::exponential(),
        256,
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    complete_registration(&mut first_server, &mut events).await;

    within(client.reconnect()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected {
            will_retry: true,
            ..
        })
    ));
    registration_burst(&mut second_server).await;
    server_line(&mut second_server, b":ergo.test CAP * LS :\r\n").await;
    client_line(&mut second_server, b"CAP END\r\n").await;
    server_line(
        &mut second_server,
        b":ergo.test 001 papo-driver :Welcome back\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Reconnected(_))));

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
        256,
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);

    complete_history_registration(&mut first_server, &mut events).await;
    client_line(&mut first_server, b"JOIN #papo\r\n").await;
    server_line(&mut first_server, b"@time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120000 :emily!~e@localhost PRIVMSG #papo :before disconnect\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    within(first_server.shutdown()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected { .. })
    ));
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
    assert!(matches!(events.next().await, Some(Event::Reconnected(_))));
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

    let Some(Event::History {
        target,
        messages,
        end,
    }) = events.next().await
    else {
        panic!("expected aggregate history event")
    };
    assert_eq!(target, "#papo");
    assert_eq!(
        messages.iter().map(MessageEvent::text).collect::<Vec<_>>(),
        ["missed one", "missed two"]
    );
    assert!(
        messages
            .iter()
            .all(|message| message.origin() == Origin::History && !message.is_live())
    );
    assert!(end);
    abort_task(driver).await;
}

#[test]
fn reconnect_history_stops_at_the_literal_per_target_limit() {
    for count in [1_000, 1_001] {
        let mut catchup = super::Catchup {
            target: "#papo".into(),
            messages: Vec::new(),
            seen: std::collections::HashSet::new(),
        };
        let next = super::extend_catchup(
            &mut catchup,
            (0..count).map(|index| {
                Message::privmsg("#papo", format!("backlog message {index}"))
                    .with_tag("msgid", format!("RfcBTzyxY12{index:04}"))
            }),
        );
        assert_eq!(catchup.messages.len(), 1_000);
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

    within(client.send(Message::new("WHOIS", ["emily"])))
        .await
        .unwrap();
    client
        .privmsg("#papo", "queued during reconnect")
        .await
        .unwrap();
    within(client.send(Message::new("MODE", ["#papo"])))
        .await
        .unwrap();
    assert!(matches!(
        within(client.privmsg("#papo", "queue overflow")).await,
        Err(Error::Disconnected)
    ));

    tokio::time::advance(Duration::from_secs(2)).await;
    registration_burst(&mut server).await;
    client_line(&mut server, b"WHOIS emily\r\n").await;
    client_line(&mut server, b"PRIVMSG #papo :queued during reconnect\r\n").await;
    client_line(&mut server, b"MODE #papo\r\n").await;
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
    within(client.reconnect()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected {
            will_retry: true,
            ..
        })
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
        256,
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
    while client.is_connected() {
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
            Box::pin(async { Err(Error::Unsupported("scripted connection failure")) })
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
        256,
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
        Err(Error::Disconnected)
    ));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected { .. })
    ));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected { .. })
    ));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected { reason, .. }) if reason == "SlowConsumer"
    ));
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
        within(driver).await.unwrap(),
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
    within(stable_server.shutdown()).await.unwrap();
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
    let (registration, waiter) = tokio::sync::oneshot::channel();
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
        256,
        Box::new(store),
        connector,
    );

    assert!(matches!(within(driver).await, Ok(())));
    assert!(matches!(
        within(waiter).await.unwrap(),
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
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected {
            reason,
            will_retry: false,
        }) if reason == "server closed the connection"
    ));
    assert!(matches!(
        within(task).await.unwrap(),
        Err(Error::Disconnected)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_error_overrides_a_non_io_terminal_cause() {
    let (mut server, _client, mut events, driver) = scripted_with_failures(false);
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected {
            reason,
            will_retry: false,
        }) if reason == "shutdown failed"
    ));
    assert!(
        matches!(within(driver).await.unwrap(), Err(Error::Io(error))
        if error.to_string() == "shutdown failed")
    );
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
    assert!(
        matches!(within(driver).await.unwrap(), Err(Error::Io(error))
        if error.to_string() == "original read failure")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn overlong_line_becomes_a_terminal_disconnect() {
    let (mut server, _client, mut events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
    within(server.write_all(&vec![b'x'; 8704])).await.unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected {
            reason,
            will_retry: false,
        }) if reason == "incoming IRC line exceeds 8703 bytes"
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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
        Some(Event::Malformed { line }) if line == b":ergo.test NOTICE * :caf\xe9"
    ));
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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
        Some(Event::AuthenticationFailed { .. })
    ));
    client_line(&mut server, b"CAP END\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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

    assert!(
        matches!(within(waiter).await.unwrap(), Err(Error::Registration(reason)) if reason.contains("SASL"))
    );
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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
        256,
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
        Some(Event::Disconnected {
            will_retry: false,
            ..
        })
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
    ));
    tokio::time::advance(Duration::from_secs(10)).await;
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[cfg(tls)]
#[test]
fn certificate_verification_failure_is_fatal() {
    let error = Error::Connect(crate::transport::ConnectError::Tls(
        rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
    ));
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
        Some(Event::Message(message)) if message.as_ref().command_is("NOTICE")
    ));
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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
        b"MODE papo-driver +B\r\nJOIN #papo\r\nJOIN #papo-bots private-channel-key\r\n",
    )
    .await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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
        Some(Event::StsUpgrade { port: 6697 })
    ));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected {
            reason,
            will_retry: false,
        }) if reason == "sts upgrade"
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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
        256,
        Box::new(papo_core::sts::MemoryStsStore::new()),
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
        256,
        Box::new(papo_core::sts::MemoryStsStore::new()),
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
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
    ));
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
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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
        256,
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
    assert!(matches!(events.next().await, Some(Event::StsPolicy(_))));
    client_line(&mut server, b"CAP END\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
    ));
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
    within(server.read_exact(&mut whois)).await.unwrap();
    assert_eq!(&whois, b"WHOIS emily\r\n");
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(task).await.unwrap(),
        Err(Error::Disconnected)
    ));
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
    let who = within(who).await.unwrap().unwrap();
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
    client_line(&mut server, b"@label=8 WHOIS missing-nick\r\n").await;
    server_line(
        &mut server,
        b"@label=8 :ergo.test 401 papo-driver missing-nick :No such nick\r\n",
    )
    .await;
    assert!(matches!(
        within(request).await.unwrap(),
        Err(Error::Server(papo_core::ServerError::Numeric { numeric, .. })) if numeric.code() == 401
    ));

    let timed_out = tokio::spawn({
        let client = client.clone();
        async move { client.raw(Message::new("WHOIS", ["alice"])).await }
    });
    client_line(&mut server, b"@label=9 WHOIS alice\r\n").await;
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        within(timed_out).await.unwrap(),
        Err(Error::Timeout)
    ));

    let aborted = tokio::spawn(async move { client.raw(Message::new("WHOIS", ["emily"])).await });
    client_line(&mut server, b"@label=A WHOIS emily\r\n").await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(aborted).await.unwrap(),
        Err(Error::Disconnected)
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn typed_requests_match_unlabelled_responses() {
    let (mut server, client, mut events, driver) =
        scripted(config().rate_limit(papo_core::RateLimit::unlimited()));
    let driver = tokio::spawn(driver);
    complete_registration(&mut server, &mut events).await;

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
    assert_eq!(within(join).await.unwrap().unwrap().members_count, 2);
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
        async move { client.mode("#papo", "+t", [] as [&str; 0]).await }
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
        async move { client.part("#papo", None).await }
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
        client
            .raw(Message::new("VERSION", Vec::<String>::new()))
            .await,
        Err(Error::Unsupported("labeled-response"))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
    ));
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
    assert!(matches!(events.next().await, Some(Event::Joined { .. })));

    let mode = tokio::spawn({
        let client = client.clone();
        async move { client.mode("#papo", "+t", [] as [&str; 0]).await }
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
        client.snapshot().channel("#papo").unwrap().mode('t'),
        Some(None)
    );
    assert!(matches!(
        events.next().await,
        Some(Event::ModeChanged { .. })
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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
        Err(Error::Disconnected)
    ));

    let parted = tokio::spawn({
        let client = client.clone();
        async move { client.join("#registered").await }
    });
    client_line(&mut server, b"@label=2 JOIN #registered\r\n").await;
    server_line(&mut server, b"@label=2 :papo-driver!papo@localhost JOIN #registered * :papo driver test\r\n:papo-driver!papo@localhost PART #registered :Registration required\r\n").await;
    assert!(matches!(
        within(parted).await.unwrap(),
        Err(Error::Disconnected)
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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
        within(client.privmsg("#papo", &"a".repeat(600)))
            .await
            .unwrap(),
        super::Sent::default()
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
        Err(Error::EmptyText)
    ));
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(task).await.unwrap(),
        Err(Error::Disconnected)
    ));
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
    within(client.privmsg("#papo", &"a".repeat(600)))
        .await
        .unwrap();
    let batch = format!(
        "BATCH +b0-0 draft/multiline #papo\r\n@batch=b0-0 PRIVMSG #papo {}\r\n@batch=b0-0;draft/multiline-concat PRIVMSG #papo {}\r\nBATCH -b0-0\r\n",
        "a".repeat(413),
        "a".repeat(187)
    );
    client_line(&mut server, batch.as_bytes()).await;
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(task).await.unwrap(),
        Err(Error::Disconnected)
    ));
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
    assert_eq!(sent.msgid.as_deref(), Some("RfcBTzyxY120004"));
    assert_eq!(
        sent.time,
        Some(UNIX_EPOCH + Duration::new(1_770_091_506, 789_000_000))
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(task).await.unwrap(),
        Err(Error::Disconnected)
    ));
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
    while !client.isupport().clienttagdeny().denies("typing") {
        tokio::task::yield_now().await;
    }
    within(client.typing("emily", Typing::Done)).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(task).await.unwrap(),
        Err(Error::Disconnected)
    ));
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
            .redact("#papo", "RfcBTzyxY120004", Some("duplicate message"))
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
    let rejected =
        tokio::spawn(async move { sender.redact("#papo", "RfcBTzyxY120005", None).await });
    client_line(&mut server, b"@label=1 REDACT #papo RfcBTzyxY120005\r\n").await;
    server_line(
        &mut server,
        b"@label=1 :ergo.test FAIL REDACT FORBIDDEN #papo :Insufficient privileges\r\n",
    )
    .await;
    assert!(matches!(
        within(rejected).await.unwrap(),
        Err(Error::Server(papo_core::ServerError::Standard { command, code, .. }))
            if command == "REDACT" && code == "FORBIDDEN"
    ));

    server_line(
        &mut server,
        b":ergo.test CAP * DEL :draft/message-redaction\r\n",
    )
    .await;
    while client.has_cap("draft/message-redaction") {
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        within(client.redact("#papo", "RfcBTzyxY120006", None)).await,
        Err(Error::Unsupported("draft/message-redaction"))
    ));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(task).await.unwrap(),
        Err(Error::Disconnected)
    ));
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
    assert!(matches!(
        within(task).await.unwrap(),
        Err(Error::Disconnected)
    ));
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
    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(task).await.unwrap(),
        Err(Error::Disconnected)
    ));
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
    within(server.write_all(b":ergo.test NOTICE papo-driver :connection closing\r\n"))
        .await
        .unwrap();

    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
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
    within(server.read_exact(&mut line)).await.unwrap();
    assert_eq!(&line, b"QUIT\r\n");
    tokio::time::advance(Duration::from_secs(9)).await;
    assert!(!quit.is_finished());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(within(quit).await.unwrap(), Err(Error::Timeout)));
    assert!(within(driver).await.unwrap().is_ok());
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
    assert!(matches!(events.next().await, Some(Event::Joined { .. })));
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
        Ok(()) | Err(Error::Disconnected)
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

    assert!(matches!(
        within(whois).await.unwrap(),
        Err(Error::Disconnected)
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
    ));
    assert_eq!(next_message(&mut events).await.text(), "maintenance soon");
    assert_eq!(
        next_message(&mut events).await.text(),
        "maintenance extended"
    );
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected { reason, .. }) if reason == "Closing Link: papo-driver (Server shutdown)"
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

    assert!(matches!(
        within(whois).await.unwrap(),
        Err(Error::Disconnected)
    ));
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
    ));
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    assert!(matches!(events.next().await, Some(Event::Message(_))));
    assert!(matches!(
        events.next().await,
        Some(Event::Disconnected { reason, .. }) if reason == "SlowConsumer"
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
    assert!(matches!(
        within(task).await.unwrap(),
        Err(Error::Disconnected)
    ));
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

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
    ));
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
        assert_eq!(state.user("ALICE").unwrap().username(), Some("~alice"));
        assert_eq!(state.monitor_online().next().unwrap().nick(), "bob");
        assert_eq!(state.isupport().nicklen(), Some(27));
    });
    let snapshot = client.snapshot();
    assert_eq!(snapshot.nick(), "papo-driver");
    assert_eq!(snapshot.account(), Some("emily"));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
    ));
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

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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

    assert!(client.has_cap("no-implicit-names"));
    assert!(client.has_cap("draft/no-implicit-names"));
    assert!(!client.has_cap("message-tags"));
    assert_eq!(client.isupport().nicklen(), Some(27));
    assert_eq!(client.casemap("#PAPO[dev]"), "#papo[dev]");
    assert_ne!(client.casemap("#PAPO[dev]"), client.casemap("#papo{dev}"));
    client.state(|state| assert!(state.has_cap("draft/no-implicit-names")));

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
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
    assert!(matches!(
        events.next().await,
        Some(Event::MonitorOnline { .. })
    ));
    assert!(matches!(
        events.next().await,
        Some(Event::MonitorOnline { .. })
    ));

    server_line(&mut server, b":ergo.test 731 papo-driver :alice\r\n").await;
    assert!(matches!(events.next().await, Some(Event::MonitorOffline { nick }) if nick == "alice"));
    let online: Vec<_> = client.state(|state| {
        state
            .monitor_online()
            .map(|user| user.nick().to_owned())
            .collect()
    });
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
    assert!(matches!(
        events.next().await,
        Some(Event::MonitorOnline { .. })
    ));
    assert_eq!(client.state(|state| state.monitor_online().count()), 1);

    within(server.shutdown()).await.unwrap();
    assert!(matches!(
        within(driver).await.unwrap(),
        Err(Error::Disconnected)
    ));
    assert_eq!(client.state(|state| state.monitor_online().count()), 0);
}
