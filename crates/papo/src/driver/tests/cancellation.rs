use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cancelling_driver_updates_the_connected_snapshot() {
    let (mut server, client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    let snapshot = client.snapshot();
    assert!(snapshot.is_connected());

    let pending = tokio::spawn(client.whois("alice"));
    client_line(&mut server, b"WHOIS alice\r\n").await;
    server_line(
        &mut server,
        b":alice!~alice@cloak.example PRIVMSG papo-driver :release is ready\r\nPING :queued-before-abort\r\n",
    )
    .await;
    client_line(&mut server, b"PONG queued-before-abort\r\n").await;

    abort_task(task).await;
    assert_eq!(within(server.read(&mut [0])).await.unwrap(), 0);
    for error in [
        within(pending).await.unwrap().unwrap_err(),
        within(client.whois("alice")).await.unwrap_err(),
        within(client.reconnect()).await.unwrap_err(),
    ] {
        assert!(matches!(error, Error::Disconnected(_)));
        assert_eq!(error.to_string(), Error::connection_lost().to_string());
    }
    assert!(within(client.quit()).await.is_ok());
    let (event, event_snapshot) = within(events.next()).await.unwrap();
    assert!(matches!(event, Event::Message(_)));
    assert!(event_snapshot.is_connected());
    assert!(within(events.next()).await.is_none());

    assert!(snapshot.is_connected());
    assert!(!client.snapshot().is_connected());
}

#[cfg(panic = "unwind")]
#[tokio::test(flavor = "current_thread")]
async fn unwinding_drops_the_driver_and_publishes_a_disconnected_snapshot() {
    let (mut server, client, mut events, driver) = scripted(config());
    let (fail, trigger) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        tokio::select! {
            result = driver => panic!("driver ended before unwinding: {result:?}"),
            result = trigger => {
                result.unwrap();
                panic!("scripted task panic");
            }
        }
    });
    complete_connect(&mut server, &mut events).await;
    assert!(client.snapshot().is_connected());

    fail.send(()).unwrap();
    let result = within(task).await.unwrap_err();
    assert_eq!(
        result.into_panic().downcast_ref::<&str>(),
        Some(&"scripted task panic")
    );
    assert_eq!(within(server.read(&mut [0])).await.unwrap(), 0);
    assert!(!client.snapshot().is_connected());
    assert!(within(events.next()).await.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_driver_directly_updates_the_connected_snapshot() {
    let (mut server, client, mut events, mut driver) = scripted(config());

    tokio::select! {
        _ = complete_connect(&mut server, &mut events) => {},
        result = &mut driver => panic!("driver ended before registration: {result:?}"),
    }
    assert!(client.snapshot().is_connected());
    drop(driver);

    assert!(!client.snapshot().is_connected());
    assert_eq!(within(server.read(&mut [0])).await.unwrap(), 0);
    assert!(within(events.next()).await.is_none());
}
