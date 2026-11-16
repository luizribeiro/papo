use super::*;

#[tokio::test(flavor = "current_thread")]
async fn plaintext_sts_policy_fails_registration_without_a_tls_provider() {
    let (stream, mut server) = tokio::io::duplex(4096);
    let stream = Arc::new(Mutex::new(Some(
        Box::new(stream) as crate::driver::BoxConnection
    )));
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let connector: crate::driver::Connector = Arc::new({
        let attempts = Arc::clone(&attempts);
        move |plan, _| {
            attempts.lock().unwrap().push(plan.address.port);
            let stream = stream.lock().unwrap().take().unwrap();
            Box::pin(async move { Ok(stream) })
        }
    });
    let (registration, waiter) = tokio::sync::oneshot::channel();
    let (_client, mut events, driver) = Driver::with_connector(
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
    welcome_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :sts=port=6699\r\n").await;

    let Some(Event::Disconnected(disconnected)) = next_event(&mut events).await else {
        panic!("expected the failed upgrade to disconnect");
    };
    assert_eq!(disconnected.reconnect_in, None);
    let crate::DisconnectCause::Connect(cause) = &disconnected.cause else {
        panic!(
            "expected a connection failure, got {:?}",
            disconnected.cause
        );
    };
    assert_eq!(cause.kind(), crate::ConnectErrorKind::HandshakeFailed);
    assert!(matches!(
        within(waiter).await.unwrap(),
        Err(Error::Connect(error)) if error.kind() == crate::ConnectErrorKind::HandshakeFailed
    ));
    assert!(within(driver).await.unwrap().is_ok());
    assert_eq!(&*attempts.lock().unwrap(), &[6667]);
}
