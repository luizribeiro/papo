//! Highest-layer coverage for peer pin conversation and server scope.

mod support {
    pub mod direct;
    pub mod peer_pin;
}

use papo::Reconnect;
use papo_testing::wire;
use support::{
    direct::connected,
    peer_pin::{assert_match, assert_mismatch, join},
};

#[tokio::test]
async fn equal_account_pins_do_not_merge_nickname_threads() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("ircs://verified.example", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["extended-join"]).await;
    connected(&mut events).await;
    for line in [
        ":papo!p@localhost JOIN #review papo-account :Papo",
        ":alice!~alice@desktop.example JOIN #review alice-account :Alice Desktop",
        ":alice-phone!~alice@phone.example JOIN #review alice-account :Alice Phone",
    ] {
        join(&mut events, &mut wire, line).await;
    }
    let desktop = client.query("alice").unwrap();
    let phone = client.query("alice-phone").unwrap();

    assert_ne!(desktop, phone);
    assert_ne!(desktop.nick().key(), phone.nick().key());
    assert_ne!(desktop.key(), phone.key());
    assert_mismatch(
        &desktop.key(),
        &phone.key(),
        papo::PeerKeyMismatch::DifferentAccount,
    );
    assert!(desktop.key().is_verified());
}

#[tokio::test]
async fn only_the_tls_authenticated_server_name_scopes_durable_pins() {
    async fn pin(
        endpoint: &str,
        server_name: Option<&str>,
        network: Option<&str>,
        bouncer_network: Option<&str>,
    ) -> papo::PeerKey {
        let mut builder = papo::Client::builder(endpoint, "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never());
        if let Some(server_name) = server_name {
            builder = builder.tls_server_name(server_name);
        }
        let (client, mut events, mut wire) = wire(builder);
        wire.register(&["extended-join"]).await;
        connected(&mut events).await;
        let features = [
            network.map(|value| format!("NETWORK={value}")),
            bouncer_network.map(|value| format!("BOUNCER_NETID={value}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if !features.is_empty() {
            wire.server(format!(
                ":offline.test 005 papo {} :are supported",
                features.join(" ")
            ))
            .await;
        }
        join(
            &mut events,
            &mut wire,
            ":papo!p@localhost JOIN #review papo-account :Papo",
        )
        .await;
        join(
            &mut events,
            &mut wire,
            ":alice!~alice@cloak.example JOIN #review alice-account :Alice",
        )
        .await;
        client.query("alice").unwrap().key()
    }

    let first = pin("ircs://verified.example", None, Some("OneName"), None).await;
    let renamed = pin("ircs://verified.example", None, Some("AnotherName"), None).await;
    let other_host = pin("ircs://other.example", None, Some("OneName"), None).await;
    let overridden = pin(
        "ircs://verified.example",
        Some("certificate.example"),
        Some("OneName"),
        None,
    )
    .await;
    let bounced = pin(
        "ircs://verified.example",
        None,
        Some("OneName"),
        Some("libera-42"),
    )
    .await;
    let plaintext = pin("irc://verified.example", None, Some("OneName"), None).await;
    assert_match(&first, &renamed);
    assert_mismatch(&first, &other_host, papo::PeerKeyMismatch::DifferentAccount);
    assert_mismatch(&first, &overridden, papo::PeerKeyMismatch::DifferentAccount);
    assert_mismatch(&first, &bounced, papo::PeerKeyMismatch::CannotVerify);
    assert!(first.is_verified());
    assert!(!bounced.is_verified());
    assert!(!plaintext.is_verified());
}
