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
        .create_account_on_connect(NewAccount::new(crate::Secret::new(
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
        .tls_ca_pem(b"test CA".to_vec())
        .tls_client_cert(
            crate::CertificateChain::new(b"certificate".to_vec()),
            crate::PrivateKey::new(b"private key".to_vec()),
        )
        .tls_server_name("chat.example.net");
    assert!(matches!(
        builder.tls.roots,
        crate::transport::TlsRoots::Ca(pem) if pem == b"test CA"
    ));
    assert!(builder.tls.client_certificate.is_some());
    assert_eq!(builder.tls.server_name.as_deref(), Some("chat.example.net"));
}

#[tokio::test(flavor = "current_thread")]
async fn build_validates_every_tls_setting_for_every_scheme() {
    const CERTIFICATE: &[u8] = include_bytes!("../../../tests/data/client.pem");
    const PRIVATE_KEY: &[u8] = include_bytes!("../../../tests/data/client.key");

    fn bad_ca(builder: Builder) -> Builder {
        builder.tls_ca_pem(b"-----BEGIN CERTIFICATE-----\nnot-base64!\n-----END CERTIFICATE-----\n")
    }
    fn bad_certificate(builder: Builder) -> Builder {
        builder.tls_client_cert(
            crate::CertificateChain::new(b"not a certificate"),
            crate::PrivateKey::new(PRIVATE_KEY),
        )
    }
    fn bad_private_key(builder: Builder) -> Builder {
        builder.tls_client_cert(
            crate::CertificateChain::new(CERTIFICATE),
            crate::PrivateKey::new(b"not a private key"),
        )
    }
    fn bad_server_name(builder: Builder) -> Builder {
        builder.tls_server_name("not a server name")
    }

    type TlsCase = (fn(Builder) -> Builder, &'static str);
    let default = Builder::new("irc://irc.example.net");
    assert!(matches!(
        default.tls.roots,
        crate::transport::TlsRoots::Platform
    ));
    assert!(!default.tls.has_custom_settings());

    let cases: [TlsCase; 4] = [
        (bad_ca, "invalid certificate PEM"),
        (bad_certificate, "certificate PEM contains no certificates"),
        (bad_private_key, "invalid client private key PEM"),
        (bad_server_name, "invalid TLS server name"),
    ];
    let schemes: &[&str] = if cfg!(feature = "websocket") {
        &["irc", "ircs", "ws", "wss"]
    } else {
        &["irc", "ircs"]
    };
    for scheme in schemes {
        for (configure, expected) in cases {
            let endpoint = format!("{scheme}://irc.example.net");
            let (_client, _events, driver) =
                configure(Builder::from_endpoint(endpoint.as_str(), "papo-client"))
                    .reconnect(crate::Reconnect::never())
                    .build();
            assert!(
                matches!(
                    within(driver).await,
                    Err(Error::InvalidInput(InvalidInput::TlsConfig(message)))
                        if message.contains(expected)
                ),
                "accepted {scheme} configuration with {expected}"
            );
        }
    }
}

#[cfg(feature = "unstable")]
#[test]
fn bouncer_networks_inherit_the_tls_server_name() {
    let (control, _events, _driver) = Builder::from_endpoint("ircs://soju.example:7443", "alice")
        .tls_server_name("bouncer.example")
        .build();

    let network = control.bouncer().network(&crate::NetworkId::from("libera"));
    assert_eq!(network.tls.server_name.as_deref(), Some("bouncer.example"));
}
