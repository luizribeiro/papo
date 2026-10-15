//! Connections to the runner-managed Ergo server.
#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use std::{io, net::ToSocketAddrs};

use papo::transport::{ConnectError, Security, Stream, connect};
use papo_test_server::from_env;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test(flavor = "current_thread")]
async fn plaintext_cap_exchange() {
    let server = from_env();
    let stream = connect(
        &server.plaintext_addr().ip().to_string(),
        server.plaintext_addr().port(),
        &Security::Plaintext,
    )
    .await
    .unwrap();
    assert!(!stream.is_secure());
    assert_cap_exchange(stream).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_closed_port_returns_connection_refused() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let error = connect("127.0.0.1", port, &Security::Plaintext)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ConnectError::Tcp(error) if error.kind() == io::ErrorKind::ConnectionRefused
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn localhost_falls_back_to_the_ipv4_listener() {
    let server = from_env();
    let addresses = ("localhost", server.plaintext_addr().port())
        .to_socket_addrs()
        .unwrap()
        .collect::<Vec<_>>();
    if !addresses.iter().any(std::net::SocketAddr::is_ipv6) {
        eprintln!("skipped: localhost has no IPv6 address");
        return;
    }
    assert!(addresses.iter().any(std::net::SocketAddr::is_ipv4));
    let stream = connect(
        "localhost",
        server.plaintext_addr().port(),
        &Security::Plaintext,
    )
    .await
    .unwrap();
    assert_cap_exchange(stream).await;
}

async fn assert_cap_exchange(mut stream: Stream) {
    stream.write_all(b"CAP LS 302\r\n").await.unwrap();
    let line = read_line(&mut stream).await.unwrap();
    assert!(line.starts_with(":ergo.test CAP * LS "), "{line:?}");
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
