use std::{env, fs, time::Duration};

use papo::{ConnectErrorKind, Endpoint, Event};
use papo_conformance_macros::scenario;

use crate::Scenario;

/// registers with the requested nickname
#[scenario(covers = ["base.connreg.nick-user"])]
async fn registration(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    assert_registered(&alice, "alice");
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}

/// registers over TLS with the configured test CA
#[scenario(covers = ["core.tls.handshake"])]
async fn tls(s: Scenario) -> papo_testing::Result<()> {
    let endpoint = Endpoint::tls("localhost", s.server.tls_addr().port());
    let ca = fs::read(s.server.ca_pem_path())?;
    register_over(&s, endpoint, Some(ca)).await
}

/// rejects a TLS certificate outside the configured trust roots
#[scenario(covers = ["core.tls.cert-verification-failure"])]
async fn tls_certificate_rejected(s: Scenario) -> papo_testing::Result<()> {
    let endpoint = Endpoint::tls("localhost", s.server.tls_addr().port());
    let Err(error) = s.connect_with(s.configure_at("alice", endpoint)).await else {
        panic!("the private test CA was trusted by default")
    };
    let error = error
        .downcast_ref::<papo::Error>()
        .expect("connection failure retains papo::Error");
    assert!(matches!(
        error,
        papo::Error::Connect(error) if error.kind() == ConnectErrorKind::CertificateRejected
    ));
    Ok(())
}

/// registers over a WebSocket transport
#[scenario(covers = [
    "core.websocket.plain-transport",
    "core.websocket.subprotocol-negotiation"
])]
async fn websocket(s: Scenario) -> papo_testing::Result<()> {
    register_over_environment(&s, "PAPO_TEST_WS", false).await
}

/// registers over a TLS WebSocket transport
#[scenario(covers = ["core.websocket.tls-transport"])]
async fn websocket_tls(s: Scenario) -> papo_testing::Result<()> {
    register_over_environment(&s, "PAPO_TEST_WSS", true).await
}

async fn register_over_environment(
    s: &Scenario,
    variable: &'static str,
    secure: bool,
) -> papo_testing::Result<()> {
    let Ok(address) = env::var(variable) else {
        return s.skip(format!("{variable} is not set up"));
    };
    if address.is_empty() {
        return s.skip(format!("{variable} is not set up"));
    }
    let ca = secure
        .then(|| fs::read(s.server.ca_pem_path()))
        .transpose()?;
    register_over(s, Endpoint::websocket(address), ca).await
}

async fn register_over(
    s: &Scenario,
    endpoint: Endpoint,
    ca: Option<Vec<u8>>,
) -> papo_testing::Result<()> {
    let mut builder = s.configure_at("alice", endpoint);
    if let Some(ca) = ca {
        builder = builder.tls_ca_pem(ca);
    }
    let mut alice = s.connect_with(builder).await?;
    assert_registered(&alice, "alice");
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}

fn assert_registered(alice: &papo_testing::Peer, label: &str) {
    let state = alice.state();
    assert!(state.is_connected());
    assert!(state.nick().starts_with(label));
    assert!(state.nick().len() > label.len());
    assert!(!state.isupport().chantypes().is_empty());
    assert!(!state.isupport().prefix().is_empty());
}

/// answers a client PING with the matching PONG
#[scenario(covers = ["base.ping.client-initiated"])]
async fn ping(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let elapsed = alice.ping().await?;
    assert!(elapsed <= Duration::from_secs(10));
    assert!(alice.state().is_connected());
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}
