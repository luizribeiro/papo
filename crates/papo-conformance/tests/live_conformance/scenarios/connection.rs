use std::{env, fs, time::Duration};

use papo::{ConnectErrorKind, Endpoint, Event, WebIrc, isupport::Advertised};
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
    assert_eq!(connected.nick, alice.snapshot().nick().as_str());
    for (name, value) in &connected.caps {
        assert_eq!(alice.snapshot().caps().get(name), Some(value));
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
        assert_eq!(alice.snapshot().caps().get(name), Some(value));
    }
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}

/// removes upstream-dependent capabilities after a bouncer network disconnects
#[scenario(covers = ["core.cap-notify.del"])]
async fn cap_notify_del(s: Scenario) -> papo_testing::Result<()> {
    if papo_test_server::DeploymentProfile::from_env()?
        != papo_test_server::DeploymentProfile::SojuErgo
    {
        return s.skip("the selected profile is not the managed soju bouncer");
    }

    let bouncer = s
        .server
        .bouncer()
        .expect("the soju profile exposes bouncer metadata");
    let network = papo_test_server::unique("cap-del");
    let login = bouncer.provision_network(&network, "papo capability observer", None)?;
    let address = s.server.plaintext_addr();
    let control = s
        .connect_with(
            papo::Client::builder(
                Endpoint::plaintext(address.ip().to_string(), address.port()).unwrap(),
                &network,
            )
            .sasl(papo::Sasl::plain(
                login.account_username(),
                papo::Secret::new(login.password()),
            ))
            .allow_insecure_auth(true)
            .enable(papo::Feature::Bouncer),
        )
        .await?;
    let id = control
        .bouncer()
        .networks()
        .await?
        .into_iter()
        .find(|candidate| candidate.name.as_deref() == Some(network.as_str()))
        .expect("the provisioned network is listed")
        .id;
    let mut bound = s.connect_with(control.bouncer().network(&id)).await?;
    let before = bound.snapshot().caps().clone();
    control
        .send_raw(papo::Line::privmsg(
            "BouncerServ",
            format!("network update {network}"),
        ))
        .await?;
    let changed = bound
        .wait_for(|event| {
            event
                .as_caps_changed()
                .filter(|changed| !changed.removed.is_empty())
        })
        .await?;
    assert!(changed.added.is_empty());
    assert!(changed.removed.keys().all(|name| before.contains_key(name)));
    assert!(
        changed
            .removed
            .keys()
            .all(|name| !bound.snapshot().caps().contains_key(name))
    );
    Ok(())
}

/// negotiates CAP 302 and observes a NAK
#[scenario(covers = [
    "base.connreg.cap-interleave",
    "ircv3.cap.ls-req-ack",
    "ircv3.cap.nak-response"
])]
async fn capability_negotiation(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let nak = alice
        .request_raw(papo::Line::new("CAP").with_params(["REQ", "papo.example/unavailable"]))
        .await?;
    assert_eq!(nak.len(), 1);
    assert_eq!(nak[0].command(), "CAP");
    assert_eq!(nak[0].get(1), Some("NAK"));
    assert_eq!(nak[0].get(2), Some("papo.example/unavailable"));
    Ok(())
}

/// marks every non-final CAP LS 302 line as continued
#[scenario(covers = ["core.cap.ls-302-multiline"])]
async fn capability_negotiation_multiline(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    // CAP framing is inspected raw because continuation markers are the behavior under test.
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
    let state = alice.snapshot();
    let support = state.isupport();
    assert!(matches!(support.get("NETWORK"), Advertised::Value(value) if !value.is_empty()));
    assert_eq!(support.get("PAPO_UNKNOWN_TOKEN"), Advertised::Absent);
    assert_eq!(
        papo_testing::nick(
            support.casemapping().expect("connection received ISUPPORT"),
            "Alice",
        ),
        "aLICE"
    );

    let prefixes = support.prefix();
    assert!(!prefixes.is_empty());
    assert!(prefixes.iter().all(|prefix| {
        support
            .statusmsg()
            .is_some_and(|set| set.contains(prefix.symbol()))
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
    let mut alice = s.connect("alice").await?;
    assert!(
        alice
            .snapshot()
            .caps()
            .contains_key(&papo::cap::EXTENDED_ISUPPORT)
    );
    assert!(!alice.snapshot().isupport().chantypes().is_empty());
    assert!(!alice.snapshot().isupport().prefix().is_empty());
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}

/// exposes a bounded HTTPS network icon URL without fetching it
#[scenario(requires(isupport = ["draft/ICON"]), covers = ["draft.network-icon"])]
async fn network_icon(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let support = alice.snapshot().isupport().clone();
    let Advertised::Value(value) = support.get("draft/ICON") else {
        panic!("draft/ICON was required but has no value")
    };
    assert!(value.starts_with("https://"));
    assert!(value.len() <= 512);
    assert_eq!(support.icon_url(64).as_deref(), Some(value));
    Ok(())
}

/// discovers the connection identity during the welcome burst
#[scenario(requires(caps = ["draft/whoami"]), covers = ["ergo.whoami"])]
async fn whoami(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    alice.join(s.channel_name()).await?;
    let state = alice.snapshot();
    assert!(state.caps().contains_key(&papo::cap::WHOAMI));
    let identity = super::user(&state, state.nick()).expect("WHOAMI supplies self state");
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
        .request_raw(papo::Line::new("WHOWAS").with_params([nick.as_str(), "1"]))
        .await?;
    let users = whowas
        .iter()
        .filter(|line| line.numeric() == Some(papo::numeric::RPL_WHOWASUSER))
        .collect::<Vec<_>>();
    assert_eq!(users.len(), 1);
    assert_eq!(
        users[0]
            .get(1)
            .expect("WHOWAS user reply includes a nickname"),
        nick.as_str()
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

/// lists linked servers or reports that LINKS is unsupported
#[scenario(covers = ["base.links.basic"])]
async fn links(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let replies = alice.request_raw(papo::Line::new("LINKS")).await?;
    if replies.iter().any(|line| line.numeric() == Some(421)) {
        assert_eq!(
            replies
                .iter()
                .filter(|line| line.numeric() == Some(421))
                .count(),
            1
        );
        return Ok(());
    }

    let rows = replies
        .iter()
        .filter(|line| line.numeric() == Some(364))
        .collect::<Vec<_>>();
    assert!(!rows.is_empty());
    for server in ["irc.papo.test", "services.papo.test"] {
        assert!(
            rows.iter()
                .any(|line| line.params().iter().any(|param| param == server)),
            "LINKS omitted {server}: {replies:?}"
        );
    }
    assert!(replies.iter().any(|line| line.numeric() == Some(365)));
    Ok(())
}

/// lists public channels through the typed LIST request
#[scenario(covers = ["base.list.basic"])]
async fn channel_list(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    set_topic_for_list(&mut alice, &channel, "release coordination").await?;

    let listed = alice.list().await?;
    let entry = listed
        .iter()
        .find(|entry| entry.name.as_str() == channel)
        .expect("LIST includes the visible channel");
    assert_eq!(entry.members_count, 1);
    assert!(entry.topic.ends_with("release coordination"));
    Ok(())
}

/// applies ELIST mask, user-count, creation-time, and topic-time filters
#[scenario(requires(isupport = ["ELIST"]), covers = ["base.list.elist-filters"])]
async fn elist_filters(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    set_topic_for_list(&mut alice, &channel, "fresh two-person channel").await?;

    let state = alice.snapshot();
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
    let mapping = alice.snapshot().casemapping();
    let listed = alice
        .list()
        .channels([
            mapping.mask(&channel).into_string(),
            mapping.mask(">1").into_string(),
            mapping.mask("<3").into_string(),
            mapping.mask("C<10").into_string(),
            mapping.mask("T<10").into_string(),
        ])
        .await?;
    let entry = listed
        .iter()
        .find(|entry| entry.name.as_str() == channel)
        .expect("ELIST includes the matching channel");
    assert_eq!(entry.members_count, 2);
    assert!(entry.topic.ends_with("fresh two-person channel"));
    Ok(())
}

async fn set_topic_for_list(
    peer: &mut crate::Peer,
    channel: &str,
    topic: &'static str,
) -> papo_testing::Result<()> {
    // LIST is the behavior under test. Send setup without a label so a server's labelled
    // TOPIC handling cannot prevent the scenario from reaching the LIST request.
    peer.send_raw(papo::Line::new("TOPIC").with_params([channel, topic]))
        .await?;
    peer.wait_for(|event| {
        event.as_topic_changed().filter(|changed| {
            changed.channel.name().as_str() == channel && changed.topic.as_deref() == Some(topic)
        })
    })
    .await?;
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
        .connect_with(
            s.configure("alice")?.webirc(
                WebIrc::new(
                    "papo-gateway",
                    papo::Secret::new(papo_test_server::WEBIRC_PASSWORD),
                    "192.0.2.42".parse()?,
                )
                .hostname(hostname)
                .flag("tls"),
            ),
        )
        .await?;
    let bob = s.connect("bob").await?;
    let whois = alice
        .whois(alice.snapshot().nick().as_str())
        .await?
        .expect("WHOIS target exists");
    let ordinary = alice
        .whois(bob.snapshot().nick().as_str())
        .await?
        .expect("WHOIS target exists");
    assert_eq!(whois.nick, *alice.snapshot().nick());
    assert_eq!(ordinary.nick, bob.snapshot().nick().as_str());
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
    assert!(
        matches!(
            error,
            papo::Error::Connect(error) if error.kind() == ConnectErrorKind::CertificateRejected
        ),
        "unexpected TLS failure: {error:?}"
    );
    Ok(())
}

/// sends the configured TLS server name in the `ClientHello`
#[scenario(covers = ["core.tls.sni"])]
async fn tls_sni(s: Scenario) -> papo_testing::Result<()> {
    process_owned::tls_sni(s).await
}

/// upgrades plaintext to TLS and persists the advertised STS policy
#[scenario(covers = ["core.sts.upgrade", "core.sts.persistence"])]
async fn sts(s: Scenario) -> papo_testing::Result<()> {
    process_owned::sts(s).await
}

/// keeps a server password off the plaintext hop while Ergo advertises STS
#[scenario(covers = ["security.adv-1.live-pass-before-sts"])]
async fn server_password_waits_for_live_sts(s: Scenario) -> papo_testing::Result<()> {
    process_owned::server_password_waits_for_live_sts(s).await
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
    register_over(s, address.parse::<Endpoint>().unwrap(), ca).await
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
    let state = alice.snapshot();
    assert!(state.is_connected());
    assert!(state.nick().as_str().starts_with(label));
    assert!(state.nick().as_str().len() > label.len());
    assert!(!state.isupport().chantypes().is_empty());
    assert!(!state.isupport().prefix().is_empty());
}

/// answers a client PING with the matching PONG
#[scenario(covers = ["base.ping.client-initiated"])]
async fn ping(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let elapsed = alice.ping().await?;
    assert!(elapsed <= Duration::from_secs(10));
    assert!(alice.snapshot().is_connected());
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}
