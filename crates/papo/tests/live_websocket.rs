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

use futures_util::{SinkExt, StreamExt};
use papo::transport::{Stream, connect_websocket};
#[cfg(tls)]
use papo::transport::{TlsConfig, TlsRoots};
use papo_test_server::from_env;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        Message,
        handshake::server::{ErrorResponse, Request, Response},
        http::StatusCode,
    },
};

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

#[tokio::test(flavor = "current_thread")]
#[allow(clippy::result_large_err)]
async fn retries_without_subprotocols_when_a_legacy_server_rejects_them() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (first, _) = listener.accept().await.unwrap();
        let error = accept_hdr_async(
            first,
            |request: &Request, _response: Response| -> Result<Response, ErrorResponse> {
                assert_eq!(request.uri().path(), "/webirc");
                assert_eq!(
                    request.headers()["Sec-WebSocket-Protocol"],
                    "binary.ircv3.net, text.ircv3.net"
                );
                Err(tokio_tungstenite::tungstenite::http::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Some("subprotocols unsupported".into()))
                    .unwrap())
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            tokio_tungstenite::tungstenite::Error::Http(response)
                if response.status() == StatusCode::BAD_REQUEST
        ));

        let (second, _) = listener.accept().await.unwrap();
        let mut socket = accept_hdr_async(
            second,
            |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
                assert_eq!(request.uri().path(), "/webirc");
                assert!(!request.headers().contains_key("Sec-WebSocket-Protocol"));
                Ok(response)
            },
        )
        .await
        .unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::text("PING :legacy")
        );
        socket
            .send(Message::text(":legacy.example PONG legacy"))
            .await
            .unwrap();
    });

    let mut stream = connect_websocket(&format!("ws://{address}/webirc"), None)
        .await
        .unwrap();
    assert_eq!(stream.websocket_subprotocol(), None);
    stream.write_all(b"PING :legacy\r\n").await.unwrap();
    assert_eq!(
        read_line(&mut stream).await.unwrap(),
        ":legacy.example PONG legacy\r\n"
    );
    server.await.unwrap();
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
