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
use std::{io, time::Duration};

use futures_util::{SinkExt, StreamExt};
use papo::transport::{ConnectError, Stream, connect_websocket};
#[cfg(tls)]
use papo::transport::{TlsConfig, TlsRoots};
use papo_test_server::from_env;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::{net::TcpListener, time::timeout};
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        Message,
        handshake::server::{ErrorResponse, Request, Response},
        http::{HeaderValue, StatusCode, header::SEC_WEBSOCKET_PROTOCOL},
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

#[tokio::test(flavor = "current_thread")]
#[allow(clippy::result_large_err)]
async fn does_not_retry_an_unrelated_handshake_failure() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (connection, _) = listener.accept().await.unwrap();
        let error = accept_hdr_async(
            connection,
            |_request: &Request, _response: Response| -> Result<Response, ErrorResponse> {
                Err(tokio_tungstenite::tungstenite::http::Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(Some("maintenance".into()))
                    .unwrap())
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            tokio_tungstenite::tungstenite::Error::Http(response)
                if response.status() == StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(
            timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });

    let error = connect_websocket(&format!("ws://{address}/maintenance"), None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ConnectError::WebSocket(tokio_tungstenite::tungstenite::Error::Http(response))
            if response.status() == StatusCode::SERVICE_UNAVAILABLE
    ));
    server.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
#[allow(clippy::result_large_err)]
async fn retries_when_a_successful_handshake_omits_the_subprotocol() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (first, _) = listener.accept().await.unwrap();
        let first = accept_hdr_async(
            first,
            |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
                assert_eq!(
                    request.headers()[SEC_WEBSOCKET_PROTOCOL],
                    "binary.ircv3.net, text.ircv3.net"
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        drop(first);

        let (second, _) = listener.accept().await.unwrap();
        let mut second = accept_hdr_async(
            second,
            |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
                assert!(!request.headers().contains_key(SEC_WEBSOCKET_PROTOCOL));
                Ok(response)
            },
        )
        .await
        .unwrap();
        assert_eq!(
            second.next().await.unwrap().unwrap(),
            Message::text("PING :no-subprotocol")
        );
    });

    let mut stream = connect_websocket(&format!("ws://{address}/legacy"), None)
        .await
        .unwrap();
    assert_eq!(stream.websocket_subprotocol(), None);
    stream.write_all(b"PING :no-subprotocol\r\n").await.unwrap();
    server.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
#[allow(clippy::result_large_err)]
async fn bounds_incoming_messages_at_the_extended_line_limit() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (connection, _) = listener.accept().await.unwrap();
        let mut socket = accept_hdr_async(
            connection,
            |request: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
                assert_eq!(
                    request.headers()[SEC_WEBSOCKET_PROTOCOL],
                    "binary.ircv3.net, text.ircv3.net"
                );
                response.headers_mut().insert(
                    SEC_WEBSOCKET_PROTOCOL,
                    HeaderValue::from_static("binary.ircv3.net"),
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        socket
            .send(Message::Ping(b"fixture-healthcheck".to_vec().into()))
            .await
            .unwrap();
        assert!(matches!(
            socket.next().await.unwrap().unwrap(),
            Message::Pong(payload) if payload.as_ref() == b"fixture-healthcheck"
        ));
        socket
            .send(Message::binary(vec![b'x'; 8191 + 510]))
            .await
            .unwrap();
        socket
            .send(Message::binary(vec![b'x'; 8191 + 510 + 1]))
            .await
            .unwrap();
    });

    let mut stream = connect_websocket(&format!("ws://{address}/limits"), None)
        .await
        .unwrap();
    let mut accepted = vec![0; 8191 + 510 + 2];
    stream.read_exact(&mut accepted).await.unwrap();
    assert!(accepted[..8191 + 510].iter().all(|byte| *byte == b'x'));
    assert_eq!(&accepted[8191 + 510..], b"\r\n");
    let error = stream.read_u8().await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    server.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rejects_a_partial_line_at_shutdown() {
    let server = from_env();
    let mut stream = connect_websocket(server.websocket_url(), None)
        .await
        .unwrap();
    stream.write_all(b"PING :unfinished").await.unwrap();
    let error = stream.shutdown().await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[tokio::test(flavor = "current_thread")]
async fn debug_redacts_buffered_wire_bytes() {
    let server = from_env();
    let mut stream = connect_websocket(server.websocket_url(), None)
        .await
        .unwrap();
    stream.write_all(b"AUTHENTICATE c2VjcmV0").await.unwrap();
    let debug = format!("{stream:?}");
    assert!(debug.contains("write_buffer_len: 21"), "{debug}");
    assert!(!debug.contains("AUTHENTICATE"), "{debug}");
    assert!(!debug.contains("c2VjcmV0"), "{debug}");
}

#[tokio::test(flavor = "current_thread")]
async fn bounds_outgoing_complete_and_partial_lines() {
    let server = from_env();
    let mut complete = connect_websocket(server.websocket_url(), None)
        .await
        .unwrap();
    let mut accepted = vec![b'x'; 8191 + 510];
    accepted.extend_from_slice(b"\r\n");
    complete.write_all(&accepted).await.unwrap();

    let mut complete = connect_websocket(server.websocket_url(), None)
        .await
        .unwrap();
    let mut rejected = vec![b'x'; 8191 + 510 + 1];
    rejected.extend_from_slice(b"\r\n");
    assert_eq!(
        complete.write_all(&rejected).await.unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );

    let mut partial = connect_websocket(server.websocket_url(), None)
        .await
        .unwrap();
    partial
        .write_all(&vec![b'x'; 8191 + 510 + 1])
        .await
        .unwrap();
    assert_eq!(
        partial.write_all(b"x").await.unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[tokio::test(flavor = "current_thread")]
#[allow(clippy::result_large_err)]
async fn text_subprotocol_uses_text_messages_and_close_is_eof() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (connection, _) = listener.accept().await.unwrap();
        let mut socket = accept_hdr_async(
            connection,
            |_request: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
                response.headers_mut().insert(
                    SEC_WEBSOCKET_PROTOCOL,
                    HeaderValue::from_static("text.ircv3.net"),
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::text("PING :utf8-✓")
        );
        socket
            .send(Message::text(":text.example PONG :utf8-✓"))
            .await
            .unwrap();
        socket.send(Message::Close(None)).await.unwrap();
    });

    let mut stream = connect_websocket(&format!("ws://{address}/text"), None)
        .await
        .unwrap();
    assert_eq!(stream.websocket_subprotocol(), Some("text.ircv3.net"));
    stream
        .write_all("PING :utf8-✓\r\n".as_bytes())
        .await
        .unwrap();
    assert_eq!(
        read_line(&mut stream).await.unwrap(),
        ":text.example PONG :utf8-✓\r\n"
    );
    assert_eq!(stream.read(&mut [0; 1]).await.unwrap(), 0);
    assert_eq!(stream.read(&mut [0; 1]).await.unwrap(), 0);
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
