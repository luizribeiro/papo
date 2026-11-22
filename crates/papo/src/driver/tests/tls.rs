use super::*;

#[test]
fn certificate_verification_failure_is_fatal() {
    let error = Error::Connect(
        crate::transport::ConnectError::Tls(rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownIssuer,
        ))
        .into(),
    );
    assert!(crate::driver::connect::is_fatal_failure(&error, None));
    let Event::Disconnected(disconnected) =
        Event::Disconnected(crate::driver::disconnected_event(&error, 1, None))
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

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn configuration_failures_are_terminal() {
    let failures = [
        Error::Connect(
            crate::transport::ConnectError::InvalidServerName("bad server name".into()).into(),
        ),
        Error::Connect(
            crate::transport::ConnectError::InvalidTlsConfiguration(
                "empty CA certificate document".into(),
            )
            .into(),
        ),
    ];

    for failure in failures {
        let calls = Arc::new(AtomicUsize::new(0));
        let connector: crate::driver::Connector = Arc::new({
            let calls = Arc::clone(&calls);
            move |_, _| {
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
            crate::driver::DEFAULT_CHANNEL_CAPACITY,
            None,
            crate::driver::PostConnect::default(),
            Reconnect::exponential(),
            Box::new(papo_core::sts::MemoryStsStore::new()),
            connector,
        );
        let driver = tokio::spawn(driver);

        assert!(matches!(
            next_event(&mut events).await,
            Some(Event::Disconnected(disconnected)) if !disconnected.is_reconnecting()
        ));
        assert!(matches!(
            within(driver).await.unwrap(),
            Err(Error::Connect(_))
        ));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn plaintext_sts_reconnects_immediately_with_tls_on_the_advertised_port() {
    let (first_stream, mut first_server) = tokio::io::duplex(4096);
    let (second_stream, mut second_server) = tokio::io::duplex(4096);
    let streams = Arc::new(Mutex::new(VecDeque::from([
        Box::new(first_stream) as crate::driver::BoxConnection,
        Box::new(second_stream) as crate::driver::BoxConnection,
    ])));
    let requested = Arc::new(Mutex::new(Vec::new()));
    let connector: crate::driver::Connector = Arc::new({
        let requested = Arc::clone(&requested);
        move |plan, _| {
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
        crate::driver::DEFAULT_CHANNEL_CAPACITY,
        Some(registration),
        crate::driver::PostConnect::default(),
        Reconnect::exponential(),
        Box::new(papo_core::sts::MemoryStsStore::new()),
        connector,
    );
    let driver = tokio::spawn(driver);
    welcome_burst(&mut first_server).await;
    server_line(&mut first_server, b":ergo.test CAP * LS :sts=port=6699\r\n").await;
    assert!(matches!(
        next_event(&mut events).await,
        Some(Event::Disconnected(crate::event::Disconnected {
            reconnect_in: Some(Duration::ZERO),
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

    let quit = tokio::spawn(async move { client.quit().await });
    client_line(&mut second_server, b"QUIT\r\n").await;
    within(second_server.shutdown()).await.unwrap();
    assert!(within(quit).await.unwrap().is_ok());
    assert!(within(driver).await.unwrap().is_ok());
}
