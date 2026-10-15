//! Connections to the runner-managed Ergo server.
#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
use std::fs;
use std::{io, net::ToSocketAddrs, time::Duration};

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
use papo::transport::{ClientCertificate, TlsConfig, TlsRoots};
use papo::transport::{ConnectError, Security, Stream, connect};
use papo_test_server::from_env;
#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
use papo_test_server::{ServerInfo, unique};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
const CLIENT_CERTIFICATE: &[u8] = include_bytes!("data/client.pem");
#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
const CLIENT_KEY: &[u8] = include_bytes!("data/client.key");
#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
const CLIENT_CERTFP: &str = "60eb8e5dbf6aa3e388743d9250b29e3862759373b4610b9ba3c0ea2330b4bfc4";

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
async fn plaintext_shutdown_reaches_ergo() {
    let server = from_env();
    let stream = connect(
        &server.plaintext_addr().ip().to_string(),
        server.plaintext_addr().port(),
        &Security::Plaintext,
    )
    .await
    .unwrap();
    assert_clean_shutdown(stream).await;
}

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
#[tokio::test(flavor = "current_thread")]
async fn tls_cap_exchange_trusts_the_test_ca() {
    let server = from_env();
    let stream = connect(
        "localhost",
        server.tls_addr().port(),
        &Security::Tls(tls_config(&server)),
    )
    .await
    .unwrap();
    assert!(stream.is_secure());
    assert_cap_exchange(stream).await;
}

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
#[tokio::test(flavor = "current_thread")]
async fn tls_shutdown_reaches_ergo() {
    let server = from_env();
    let stream = connect(
        "localhost",
        server.tls_addr().port(),
        &Security::Tls(tls_config(&server)),
    )
    .await
    .unwrap();
    assert_clean_shutdown(stream).await;
}

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
#[tokio::test(flavor = "current_thread")]
async fn tls_can_trust_only_the_test_ca() {
    let server = from_env();
    let config = TlsConfig {
        roots: TlsRoots::Ca(fs::read(server.ca_pem_path()).unwrap()),
        ..TlsConfig::default()
    };
    let stream = connect(
        "localhost",
        server.tls_addr().port(),
        &Security::Tls(config),
    )
    .await
    .unwrap();
    assert_cap_exchange(stream).await;
}

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
#[tokio::test(flavor = "current_thread")]
async fn tls_rejects_the_untrusted_test_ca() {
    let server = from_env();
    let error = connect(
        "localhost",
        server.tls_addr().port(),
        &Security::Tls(TlsConfig::default()),
    )
    .await
    .unwrap_err();
    assert_certificate_error(&error);
}

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
#[tokio::test(flavor = "current_thread")]
async fn tls_rejects_a_wrong_server_name() {
    let server = from_env();
    let mut config = tls_config(&server);
    config.server_name = Some("wrong.example".into());
    let error = connect(
        &server.tls_addr().ip().to_string(),
        server.tls_addr().port(),
        &Security::Tls(config),
    )
    .await
    .unwrap_err();
    assert_certificate_error(&error);
}

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
#[tokio::test(flavor = "current_thread")]
async fn tls_presents_a_client_certificate_seen_by_ergo() {
    let server = from_env();
    let mut config = tls_config(&server);
    config.client_certificate = Some(ClientCertificate::new(CLIENT_CERTIFICATE, CLIENT_KEY));
    let mut stream = connect(
        "localhost",
        server.tls_addr().port(),
        &Security::Tls(config),
    )
    .await
    .unwrap();
    let nick = unique("papo-certfp");
    stream
        .write_all(format!("NICK {nick}\r\nUSER papo 0 * :papo transport test\r\n").as_bytes())
        .await
        .unwrap();
    read_until(&mut stream, |line| line.contains(&format!(" 001 {nick} "))).await;
    stream
        .write_all(format!("WHOIS {nick}\r\n").as_bytes())
        .await
        .unwrap();
    read_until(&mut stream, |line| {
        line.contains(&format!(" 276 {nick} {nick} ")) && line.contains(CLIENT_CERTFP)
    })
    .await;
}

#[cfg(not(any(feature = "tls-ring", feature = "tls-aws-lc")))]
#[tokio::test(flavor = "current_thread")]
async fn tls_without_a_provider_returns_a_clear_error() {
    let error = connect(
        "name-does-not-exist.invalid",
        6697,
        &Security::Tls(papo::transport::TlsConfig::default()),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ConnectError::TlsUnavailable));
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

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
fn tls_config(server: &ServerInfo) -> TlsConfig {
    TlsConfig {
        roots: TlsRoots::PlatformAndCa(fs::read(server.ca_pem_path()).unwrap()),
        client_certificate: None,
        server_name: None,
    }
}

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
fn assert_certificate_error(error: &ConnectError) {
    assert!(matches!(
        error,
        ConnectError::Tls(rustls::Error::InvalidCertificate(_))
    ));
}

async fn assert_cap_exchange(mut stream: Stream) {
    stream.write_all(b"CAP LS 302\r\n").await.unwrap();
    let line = read_line(&mut stream).await.unwrap();
    assert!(line.starts_with(":ergo.test CAP * LS "), "{line:?}");
}

async fn assert_clean_shutdown(mut stream: Stream) {
    stream.write_all(b"CAP LS 302\r\n").await.unwrap();
    stream.flush().await.unwrap();
    stream.shutdown().await.unwrap();
    let reply = timeout(Duration::from_secs(2), async {
        let mut reply = Vec::new();
        loop {
            let mut chunk = [0; 1024];
            let count = stream.read(&mut chunk).await?;
            if count == 0 {
                return Ok::<_, io::Error>(reply);
            }
            if reply.len() + count > 8191 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Ergo shutdown reply exceeded 8191 bytes",
                ));
            }
            reply.extend_from_slice(&chunk[..count]);
        }
    })
    .await
    .expect("Ergo did not observe the shutdown")
    .unwrap();
    assert!(
        String::from_utf8_lossy(&reply).contains(":ergo.test CAP * LS "),
        "{reply:?}"
    );
}

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
async fn read_until(stream: &mut Stream, accepted: impl Fn(&str) -> bool) {
    for _ in 0..128 {
        let line = read_line(stream).await.unwrap();
        if accepted(&line) {
            return;
        }
    }
    panic!("Ergo did not send the expected line");
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
