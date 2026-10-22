use std::{env, fs, time::Duration};

use papo::{ConnectErrorKind, Endpoint, Event, ListFilter, WebIrc, isupport::Advertised};
use papo_conformance_macros::scenario;

use crate::Scenario;

#[cfg_attr(not(target_family = "wasm"), path = "connection/process_owned.rs")]
#[cfg_attr(target_family = "wasm", path = "connection/process_owned_wasm.rs")]
mod process_owned;

/// registers with the requested nickname
#[scenario(covers = ["base.connreg.nick-user", "base.connreg.welcome-order"])]
async fn connect(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    assert_connected(&alice, "alice");
    let connected = alice.wait_for(Event::as_connected).await?;
    assert_eq!(connected.nick, alice.state().nick().as_ref());
    for (name, value) in &connected.caps {
        assert_eq!(alice.state().caps().get(name), Some(value));
    }
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}

/// applies a capability added after connecting
#[scenario(
    requires(caps = ["soju.im/bouncer-networks"]),
    covers = ["core.cap-notify.new"]
)]
async fn cap_notify_new(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s
        .connect_with(s.configure("alice")?.enable(papo::Feature::Bouncer))
        .await?;
    let connected = alice.wait_for(Event::as_connected).await?;
    let changed = alice
        .wait_for(|event| {
            event
                .as_caps_changed()
                .filter(|changed| !changed.added.is_empty())
        })
        .await?;
    assert!(changed.removed.is_empty());
    assert!(
        changed
            .added
            .keys()
            .any(|name| !connected.caps.contains_key(name))
    );
    for (name, value) in &changed.added {
        assert_eq!(alice.state().caps().get(name), Some(value));
    }
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
    let lines = alice
        .request_raw(papo::Line::new("CAP").with_params(["LS", "302"]))
        .await?;
    if lines.len() < 2 {
        return s.skip("the server's CAP LS reply fits on one line");
    }
    assert!(lines.iter().all(|line| line.command() == "CAP"));
    assert!(
        lines[..lines.len() - 1]
            .iter()
            .all(|line| line.get(1) == Some("LS") && line.get(2) == Some("*"))
    );
    let last = lines.last().expect("CAP LS returned at least two lines");
    assert_eq!(last.get(1), Some("LS"));
    assert_ne!(last.get(2), Some("*"));

    let nak = alice
        .request_raw(papo::Line::new("CAP").with_params(["REQ", "papo.example/unavailable"]))
        .await?;
    assert_eq!(nak.len(), 1);
    assert_eq!(nak[0].command(), "CAP");
    assert_eq!(nak[0].get(1), Some("NAK"));
    assert_eq!(nak[0].get(2), Some("papo.example/unavailable"));
    Ok(())
}

/// exposes parsed ISUPPORT semantics
#[scenario(covers = [
    "base.isupport.token-parsing",
    "base.isupport.casemapping",
    "base.isupport.prefix",
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
    assert_eq!(support.casemapping().nick("Alice"), "aLICE");

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

/// negotiates extended ISUPPORT before the connection completes
#[scenario(
    requires(caps = ["draft/extended-isupport"]),
    covers = ["draft.extended-isupport"]
)]
async fn extended_isupport(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s
        .connect_with(
            s.configure("alice")?
                .request_caps(["draft/extended-isupport"]),
        )
        .await?;
    assert!(
        alice
            .state()
            .caps()
            .contains_key(&papo::cap::EXTENDED_ISUPPORT)
    );
    assert!(!alice.state().isupport().chantypes().is_empty());
    assert!(!alice.state().isupport().prefix().is_empty());
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}

/// discovers the connection identity during the welcome burst
#[scenario(requires(caps = ["draft/whoami"]), covers = ["ergo.whoami"])]
async fn whoami(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    alice.join(s.channel_name()).await?;
    let state = alice.state();
    assert!(state.caps().contains_key(&papo::cap::WHOAMI));
    let identity = state
        .user(state.nick())
        .expect("WHOAMI supplies self state");
    assert!(identity.username().is_some_and(|user| !user.is_empty()));
    assert!(identity.host().is_some_and(|host| !host.is_empty()));
    assert_eq!(identity.realname(), Some("papo test peer"));
    Ok(())
}

/// returns complete MOTD and LUSERS reply sequences
#[scenario(covers = ["base.motd.full", "base.motd.absent", "base.lusers.basic"])]
async fn server_information(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    // Numeric replies are inspected raw because their sequence is the behavior under test.
    let motd = alice.request_raw(papo::Line::new("MOTD")).await?;
    let motd_codes = motd
        .iter()
        .filter_map(papo::Line::numeric)
        .collect::<Vec<_>>();
    match motd_codes.first() {
        Some(375) => {
            assert!(motd_codes.contains(&372));
            assert_eq!(motd_codes.last(), Some(&376));
        }
        Some(422) => assert_eq!(motd_codes, [422]),
        other => panic!("unexpected MOTD start: {other:?}"),
    }

    let lusers = alice.request_raw(papo::Line::new("LUSERS")).await?;
    let codes = lusers
        .iter()
        .filter_map(papo::Line::numeric)
        .collect::<Vec<_>>();
    assert_eq!(codes.first(), Some(&251));
    assert!(codes.contains(&255));
    assert!(codes.contains(&265));
    assert!(codes.contains(&266));
    Ok(())
}

/// returns TIME and bounded WHOWAS replies
#[scenario(covers = [
    "base.time.basic",
    "base.whowas.basic",
    "base.whowas.err-no-such-nick",
    "base.whowas.count-param"
])]
async fn server_queries(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;

    // These commands expose protocol numerics rather than richer result types.
    let time = bob.request_raw(papo::Line::new("TIME")).await?;
    let reply = time
        .iter()
        .find(|line| line.numeric() == Some(papo::numeric::RPL_TIME))
        .expect("TIME returns numeric 391");
    assert!(reply.get(1).is_some_and(|server| !server.is_empty()));
    assert!(reply.get(2).is_some_and(|time| !time.is_empty()));

    let nick = alice.nick();
    alice.quit("record this departure").await?;
    let whowas = bob
        .request_raw(papo::Line::new("WHOWAS").with_params([nick.as_ref(), "1"]))
        .await?;
    let users = whowas
        .iter()
        .filter(|line| line.numeric() == Some(papo::numeric::RPL_WHOWASUSER))
        .collect::<Vec<_>>();
    assert_eq!(users.len(), 1);
    assert_eq!(
        bob.state().nickname(
            users[0]
                .get(1)
                .expect("WHOWAS user reply includes a nickname")
        ),
        nick
    );
    assert!(
        whowas
            .iter()
            .any(|line| line.numeric() == Some(papo::numeric::RPL_ENDOFWHOWAS))
    );

    let missing = papo_test_server::unique("missing-whowas");
    let absent = bob
        .request_raw(papo::Line::new("WHOWAS").with_params([&missing]))
        .await?;
    assert!(
        absent
            .iter()
            .any(|line| line.numeric() == Some(papo::numeric::ERR_WASNOSUCHNICK))
    );
    Ok(())
}

/// lists public channels through the typed LIST request
#[scenario(covers = ["base.list.basic"])]
async fn channel_list(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    alice.set_topic(&channel, "release coordination").await?;

    let listed = alice.list(ListFilter::default()).await?;
    let entry = listed
        .iter()
        .find(|entry| entry.name.as_ref() == channel)
        .expect("LIST includes the visible channel");
    assert_eq!(entry.members_count, 1);
    assert!(entry.topic.ends_with("release coordination"));
    Ok(())
}

/// applies ELIST mask, user-count, creation-time, and topic-time filters
#[scenario(requires(isupport = ["ELIST"]), covers = ["base.list.elist-filters"])]
async fn elist_filters(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    alice
        .set_topic(&channel, "fresh two-person channel")
        .await?;

    let state = alice.state();
    let filters = state.isupport().get("ELIST");
    let Advertised::Value(filters) = filters else {
        return s.skip("ELIST does not advertise filter letters");
    };
    if !['C', 'M', 'N', 'T']
        .iter()
        .all(|filter| filters.contains(*filter))
    {
        return s.skip("ELIST does not support mask, count, creation, and topic filters");
    }
    let mapping = alice.state().casemapping();
    let listed = alice
        .list(ListFilter::default().channels([
            mapping.mask(&channel),
            mapping.mask(">1"),
            mapping.mask("<3"),
            mapping.mask("C<10"),
            mapping.mask("T<10"),
        ]))
        .await?;
    let entry = listed
        .iter()
        .find(|entry| entry.name.as_ref() == channel)
        .expect("ELIST includes the matching channel");
    assert_eq!(entry.members_count, 2);
    assert!(entry.topic.ends_with("fresh two-person channel"));
    Ok(())
}

/// lists, adds, changes, and removes a bouncer network
#[scenario(
    requires(caps = ["soju.im/bouncer-networks"]),
    covers = ["base.connreg.pass", "draft.bouncer-networks"]
)]
async fn bouncer_networks(s: Scenario) -> papo_testing::Result<()> {
    process_owned::bouncer_networks(s).await
}

/// registers through a trusted WEBIRC gateway identity
#[scenario(covers = ["core.webirc.gateway"])]
async fn webirc(s: Scenario) -> papo_testing::Result<()> {
    let profile = env::var("PAPO_TEST_PROFILE").unwrap_or_default();
    if !profile.starts_with("ergo-") {
        return s.skip("the selected profile has no managed WEBIRC gateway");
    }
    let hostname = "client.papo.test";
    let mut alice = s
        .connect_with(s.configure("alice")?.webirc(WebIrc {
            password: papo_test_server::WEBIRC_PASSWORD.into(),
            gateway: "papo-gateway".into(),
            client_ip: "192.0.2.42".parse()?,
            client_hostname: Some(hostname.into()),
            options: vec![("tls".into(), None)],
        }))
        .await?;
    let bob = s.connect("bob").await?;
    let whois = alice.whois(alice.state().nick().clone()).await?;
    let ordinary = alice.whois(bob.state().nick().clone()).await?;
    assert_eq!(whois.nick, *alice.state().nick());
    assert_eq!(ordinary.nick, *bob.state().nick());
    assert!(whois.host.is_some());
    assert_ne!(whois.host, ordinary.host);
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}

/// registers over TLS with the configured test CA
#[scenario(covers = ["core.tls.handshake"])]
async fn tls(s: Scenario) -> papo_testing::Result<()> {
    let endpoint = Endpoint::tls("localhost", s.server.tls_addr().port()).unwrap();
    let ca = fs::read(s.server.ca_pem_path())?;
    register_over(&s, endpoint, Some(ca)).await
}

/// rejects a TLS certificate outside the configured trust roots
#[scenario(covers = ["core.tls.cert-verification-failure"])]
async fn tls_certificate_rejected(s: Scenario) -> papo_testing::Result<()> {
    let endpoint = Endpoint::tls("localhost", s.server.tls_addr().port()).unwrap();
    let Err(error) = s.connect_with(s.configure_at("alice", endpoint)?).await else {
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

/// upgrades plaintext to TLS and persists the advertised STS policy
#[scenario(covers = ["core.sts.upgrade", "core.sts.persistence"])]
async fn sts(s: Scenario) -> papo_testing::Result<()> {
    process_owned::sts(s).await
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
    register_over(s, Endpoint::websocket(address).unwrap(), ca).await
}

async fn register_over(
    s: &Scenario,
    endpoint: Endpoint,
    ca: Option<Vec<u8>>,
) -> papo_testing::Result<()> {
    let mut builder = s.configure_at("alice", endpoint)?;
    if let Some(ca) = ca {
        builder = builder.tls_ca_pem(ca);
    }
    let mut alice = s.connect_with(builder).await?;
    assert_connected(&alice, "alice");
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}

fn assert_connected(alice: &crate::Peer, label: &str) {
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
