use super::*;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn connect_timeout_covers_a_silent_tls_peer() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let connection = tokio::spawn(
        Builder::new(&format!("ircs://{address}"))
            .connect_timeout(Duration::from_secs(17))
            .connect(),
    );

    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(16)).await;
    assert!(!connection.is_finished());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        connection.await.unwrap(),
        Err(Error::Connect(error)) if error.kind() == ConnectErrorKind::TimedOut
    ));
    peer.abort();
}
