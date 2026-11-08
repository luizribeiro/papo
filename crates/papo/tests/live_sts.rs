//! STS upgrade behavior against a process-owned Ergo server.
#![cfg(all(not(target_family = "wasm"), tls))]

mod support;

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use support::within;

use papo::{Client, ConnectErrorKind, Endpoint, Error, Event, Reconnect};
use papo_test_server::{TestServer, unique};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test(flavor = "current_thread")]
async fn connect_waits_for_registration_after_an_sts_upgrade() {
    within(async {
        let server = TestServer::start_with_sts().unwrap();
        let nick = unique("papo-sts");
        let (client, mut events) = Client::builder(
            Endpoint::plaintext("localhost", server.plaintext_addr().port()).unwrap(),
            support::nick(&nick),
        )
        .tls_ca_pem(std::fs::read(server.ca_pem_path()).unwrap())
        .connect()
        .await
        .unwrap();

        assert!(client.snapshot().is_connected());
        while let Some(event) = events.next().await {
            if matches!(event, Event::Connected(ref welcome) if welcome.nick == nick.as_str()) {
                client.quit("STS upgrade complete").await.unwrap();
                return;
            }
        }
        panic!("driver stopped before the registered event");
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn sts_tls_handshake_failure_stops_reconnection() {
    within(async {
        let plaintext = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tls = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tls_port = tls.local_addr().unwrap().port();
        let plaintext_port = plaintext.local_addr().unwrap().port();
        let tls_attempts = Arc::new(AtomicUsize::new(0));

        let plaintext_server = tokio::spawn(async move {
            let (mut stream, _) = plaintext.accept().await.unwrap();
            let mut input = [0; 256];
            let read = stream.read(&mut input).await.unwrap();
            assert!(input[..read].starts_with(b"CAP LS 302\r\n"));
            stream
                .write_all(
                    format!(":script.test CAP * LS :sts=port={tls_port},duration=3600\r\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
        });
        let observed_attempts = Arc::clone(&tls_attempts);
        let tls_server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = tls.accept().await.unwrap();
                observed_attempts.fetch_add(1, Ordering::Relaxed);
                stream
                    .write_all(b"HTTP/1.1 400 TLS required\r\n\r\n")
                    .await
                    .unwrap();
            }
        });

        let (_client, _events, driver) = Client::builder(
            Endpoint::plaintext("127.0.0.1", plaintext_port).unwrap(),
            "papo-sts-failure",
        )
        .reconnect(
            Reconnect::exponential()
                .initial_delay(Duration::ZERO)
                .max_delay(Duration::ZERO)
                .max_attempts(4),
        )
        .build();
        let error = driver.await.unwrap_err();

        assert!(matches!(
            error,
            Error::Connect(ref error) if error.kind() == ConnectErrorKind::HandshakeFailed
        ));
        assert_eq!(tls_attempts.load(Ordering::Relaxed), 1);
        plaintext_server.await.unwrap();
        tls_server.abort();
    })
    .await;
}
