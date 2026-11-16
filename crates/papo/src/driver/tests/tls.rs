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
