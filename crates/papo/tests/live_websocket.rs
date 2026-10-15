//! WebSocket connections to the runner-managed Ergo server.
#![cfg(all(
    feature = "websocket",
    any(
        not(target_family = "wasm"),
        all(target_os = "wasi", not(target_env = "p1"))
    )
))]

#[cfg(tls)]
use std::fs;
use std::io;

use papo::transport::{Stream, connect_websocket};
#[cfg(tls)]
use papo::transport::{TlsConfig, TlsRoots};
use papo_test_server::from_env;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test(flavor = "current_thread")]
async fn plaintext_cap_exchange_prefers_binary_messages() {
    let server = from_env();
    let mut stream = connect_websocket(server.websocket_url(), None)
        .await
        .unwrap();
    assert!(!stream.is_secure());
    assert_eq!(stream.websocket_subprotocol(), Some("binary.ircv3.net"));
    stream.write_all(b"CAP LS 302\r\n").await.unwrap();
    let line = read_line(&mut stream).await.unwrap();
    assert!(line.starts_with(":ergo.test CAP * LS "), "{line:?}");
}

#[cfg(tls)]
#[tokio::test(flavor = "current_thread")]
async fn secure_websocket_trusts_the_test_ca() {
    let server = from_env();
    let config = TlsConfig {
        roots: TlsRoots::Ca(fs::read(server.ca_pem_path()).unwrap()),
        ..TlsConfig::default()
    };
    let stream = connect_websocket(server.secure_websocket_url(), Some(&config))
        .await
        .unwrap();
    assert!(stream.is_secure());
    assert_eq!(stream.websocket_subprotocol(), Some("binary.ircv3.net"));
}

#[tokio::test(flavor = "current_thread")]
async fn writes_each_complete_line_as_one_message() {
    let server = from_env();
    let mut stream = connect_websocket(server.websocket_url(), None)
        .await
        .unwrap();
    stream
        .write_all(b"PING :first\r\nPING :second\r\n")
        .await
        .unwrap();
    assert!(read_line(&mut stream).await.unwrap().contains(" PONG "));
    assert!(read_line(&mut stream).await.unwrap().contains(" PONG "));

    stream.write_all(b"PING :split").await.unwrap();
    stream.write_all(b"\r\n").await.unwrap();
    assert!(read_line(&mut stream).await.unwrap().contains(" PONG "));
}

async fn read_line(stream: &mut Stream) -> io::Result<String> {
    let mut line = Vec::new();
    for _ in 0..8191 {
        line.push(stream.read_u8().await?);
        if line.ends_with(b"\r\n") {
            return String::from_utf8(line)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "Ergo line exceeded 8191 bytes",
    ))
}
