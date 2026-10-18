use std::{env, fs, time::Duration};

use papo::{ConnectErrorKind, Endpoint, Event, isupport::Advertised};
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

/// negotiates multiline CAP 302 and observes a NAK
#[scenario(covers = [
    "base.connreg.cap-interleave",
    "ircv3.cap.ls-req-ack",
    "ircv3.cap.nak-response",
    "core.cap.ls-302-multiline"
])]
async fn capability_negotiation(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    // CAP framing is inspected raw because continuation markers and NAK are the behavior under test.
    let lines = alice.request_command("CAP", ["LS", "302"]).await?;
    if lines.len() < 2 {
        return s.skip("the server's CAP LS reply fits on one line");
    }
    assert!(lines.iter().all(|line| line.command() == "CAP"));
    assert!(
        lines[..lines.len() - 1]
            .iter()
            .all(|line| line.param(1) == Some("LS") && line.param(2) == Some("*"))
    );
    let last = lines.last().expect("CAP LS returned at least two lines");
    assert_eq!(last.param(1), Some("LS"));
    assert_ne!(last.param(2), Some("*"));

    let nak = alice
        .request_command("CAP", ["REQ", "papo.example/unavailable"])
        .await?;
    assert_eq!(nak.len(), 1);
    assert_eq!(nak[0].command(), "CAP");
    assert_eq!(nak[0].param(1), Some("NAK"));
    assert_eq!(nak[0].param(2), Some("papo.example/unavailable"));
    Ok(())
}

/// exposes parsed ISUPPORT semantics
#[scenario(covers = [
    "base.isupport.token-parsing",
    "base.isupport.casemapping",
    "base.isupport.chanmodes-groups",
    "base.isupport.targmax",
    "base.isupport.statusmsg-token"
])]
async fn isupport(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let state = alice.state();
    let support = state.isupport();
    assert!(matches!(support.get("NETWORK"), Advertised::Value(value) if !value.is_empty()));
    assert_eq!(support.get("PAPO_UNKNOWN_TOKEN"), Advertised::Absent);
    assert!(support.casemapping().eq("Alice", "aLICE"));

    let prefixes = support.prefix();
    assert!(!prefixes.is_empty());
    assert!(prefixes.iter().enumerate().all(|(rank, prefix)| {
        prefix.rank == rank
            && support
                .statusmsg()
                .is_some_and(|set| set.contains(prefix.symbol))
    }));
    let modes = support.chanmodes().expect("CHANMODES has four groups");
    assert!(!modes.list.is_empty());
    assert!(!modes.never.is_empty());
    assert!(!support.targmax().is_empty());
    Ok(())
}

/// returns complete MOTD and LUSERS reply sequences
#[scenario(covers = ["base.motd.full", "base.motd.absent", "base.lusers.basic"])]
async fn server_information(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    // Numeric replies are inspected raw because their sequence is the behavior under test.
    let motd = alice
        .request_command("MOTD", std::iter::empty::<&str>())
        .await?;
    let motd_codes = motd
        .iter()
        .filter_map(papo::RawLine::numeric)
        .collect::<Vec<_>>();
    match motd_codes.first() {
        Some(375) => {
            assert!(motd_codes.contains(&372));
            assert_eq!(motd_codes.last(), Some(&376));
        }
        Some(422) => assert_eq!(motd_codes, [422]),
        other => panic!("unexpected MOTD start: {other:?}"),
    }

    let lusers = alice
        .request_command("LUSERS", std::iter::empty::<&str>())
        .await?;
    let codes = lusers
        .iter()
        .filter_map(papo::RawLine::numeric)
        .collect::<Vec<_>>();
    assert_eq!(codes.first(), Some(&251));
    assert!(codes.contains(&255));
    assert!(codes.contains(&265));
    assert_eq!(codes.last(), Some(&266));
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
