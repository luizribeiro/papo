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

#[test]
fn debug_is_opaque_with_every_builder_secret() {
    let builder = Builder::new("ircs://irc.example.net")
        .password(crate::Secret::new("unique-network-password-9134"))
        .webirc(webirc("papo-gateway", "unique-webirc-password-3186"))
        .sasl(Sasl::plain(
            "alice",
            crate::Secret::new("unique-sasl-password-8527"),
        ))
        .create_account(NewAccount::new(crate::Secret::new(
            "unique-account-password-7416",
        )))
        .tls_client_cert(
            crate::CertificateChain::new(Vec::new()),
            crate::PrivateKey::new(b"unique-tls-key-6305".to_vec()),
        )
        .autojoin_with_key("#staff", crate::Secret::new("unique-channel-key-5294"));
    let output = format!("{builder:?}");
    assert_eq!(output, "Builder { .. }");
    for secret in [
        "unique-network-password-9134",
        "unique-webirc-password-3186",
        "unique-sasl-password-8527",
        "unique-account-password-7416",
        "unique-tls-key-6305",
        "unique-channel-key-5294",
    ] {
        assert!(!output.contains(secret));
    }

    let builder = Builder::new("ircs://irc.example.net")
        .tls_ca_pem(b"replaced CA".to_vec())
        .tls_extra_ca_pem(b"test CA".to_vec())
        .tls_client_cert(
            crate::CertificateChain::new(b"certificate".to_vec()),
            crate::PrivateKey::new(b"private key".to_vec()),
        )
        .tls_server_name("chat.example.net");
    assert!(matches!(
        builder.tls.roots,
        crate::transport::TlsRoots::PlatformAndCa(pem) if pem == b"test CA"
    ));
    assert!(builder.tls.client_certificate.is_some());
    assert_eq!(builder.tls.server_name.as_deref(), Some("chat.example.net"));

    let builder = Builder::new("ircs://irc.example.net")
        .tls_extra_ca_pem(b"replaced extra CA".to_vec())
        .tls_ca_pem(b"only CA".to_vec());
    assert!(matches!(
        builder.tls.roots,
        crate::transport::TlsRoots::Ca(pem) if pem == b"only CA"
    ));
}
