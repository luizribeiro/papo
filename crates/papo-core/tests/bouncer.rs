//! Transcript tests for upstream bouncer network management.
#![cfg(feature = "unstable")]

mod support;

use std::time::{Duration, Instant};

use papo_core::{Config, Event, NetworkConfig, NetworkId, StateLimit};
use support::Transcript;

fn registered() -> Transcript {
    Transcript::new(Config::default().wanted_caps([
        "batch",
        "labeled-response",
        "soju.im/bouncer-networks",
    ]))
    .connect_default()
    .server("CAP * LS :batch labeled-response soju.im/bouncer-networks")
    .client("CAP REQ :batch labeled-response soju.im/bouncer-networks")
    .server("CAP * ACK :batch labeled-response soju.im/bouncer-networks")
    .client("CAP END")
}

#[test]
fn binds_the_configured_network_before_cap_end() {
    let mut config = Config::default().wanted_caps(["soju.im/bouncer-networks"]);
    config.bouncer_network = Some(NetworkId::new("42"));
    Transcript::new(config)
        .connect_default()
        .server("CAP * LS :soju.im/bouncer-networks")
        .client("CAP REQ :soju.im/bouncer-networks")
        .server("CAP * ACK :soju.im/bouncer-networks")
        .inspect(|session| assert_eq!(session.supports(papo_core::Feature::Bouncer), Ok(())))
        .client("BOUNCER BIND 42")
        .client("CAP END");
}

#[test]
fn sends_each_bouncer_network_operation() {
    let config = NetworkConfig::new("irc.libera.chat")
        .name("Libera Chat")
        .port(6697)
        .tls(true)
        .nickname("alice");
    registered()
        .update(|session| {
            session
                .bouncer_networks(std::time::Instant::now())
                .unwrap();
        })
        .client("@label=0 BOUNCER LISTNETWORKS")
        .server("@label=0 BATCH +networks soju.im/bouncer-networks")
        .server(r"@batch=networks BOUNCER NETWORK 42 name=Libera\sChat;state=connected")
        .server("BATCH -networks")
        .event(|event| matches!(event, Event::BouncerNetwork { network: Some(network), .. }
            if network.name.as_deref() == Some("Libera Chat")))
        .event(|event| matches!(event, Event::Response(response) if response.lines.len() == 1))
        .update(|session| {
            session
                .bouncer_add(std::time::Instant::now(), &config)
                .unwrap();
        })
        .client(r"@label=1 BOUNCER ADDNETWORK host=irc.libera.chat;name=Libera\sChat;port=6697;tls=1;nickname=alice")
        .server("@label=1 BOUNCER ADDNETWORK 43")
        .event(|event| matches!(event, Event::Response(response) if response.lines.len() == 1))
        .update(|session| {
            session
                .bouncer_change(std::time::Instant::now(), &NetworkId::new("43"), &config)
                .unwrap();
        })
        .client(r"@label=2 BOUNCER CHANGENETWORK 43 host=irc.libera.chat;name=Libera\sChat;port=6697;tls=1;nickname=alice")
        .server("@label=2 BOUNCER CHANGENETWORK 43")
        .event(|event| matches!(event, Event::Response(response) if response.lines.len() == 1))
        .update(|session| {
            session
                .bouncer_remove(std::time::Instant::now(), &NetworkId::new("43"))
                .unwrap();
        })
        .client("@label=3 BOUNCER DELNETWORK 43")
        .server("@label=3 BOUNCER DELNETWORK 43")
        .event(|event| matches!(event, Event::Response(response) if response.lines.len() == 1))
        .no_event();
}

#[test]
fn merges_bounded_notification_batches_and_deletes_networks() {
    registered()
        .server("BATCH +notify soju.im/bouncer-networks")
        .server(r"@batch=notify BOUNCER NETWORK 42 name=My\sNetwork;host=irc.example.org;state=connecting")
        .server("BATCH -notify")
        .event(|event| matches!(event, Event::BouncerNetwork { network: Some(network), .. }
            if network.host.as_deref() == Some("irc.example.org")))
        .server("BOUNCER NETWORK 42 state=connected;host")
        .event(|event| matches!(event, Event::BouncerNetwork { network: Some(network), .. }
            if network.host.is_none()))
        .inspect(|session| {
            let network = session.state().bouncer_networks().next().unwrap();
            assert_eq!(network.name.as_deref(), Some("My Network"));
        })
        .server("BOUNCER NETWORK 42 *")
        .event(|event| matches!(event, Event::BouncerNetwork { id, network: None }
            if id.as_str() == "42"))
        .inspect(|session| assert_eq!(session.state().bouncer_networks().count(), 0))
        .no_event();
}

#[test]
fn bounds_the_server_driven_network_cache() {
    let mut transcript = registered();
    let start = Instant::now();
    for index in 0..10_001 {
        transcript = transcript.server(&format!(
            "BOUNCER NETWORK {index} host=irc{index}.example.org;state=disconnected"
        ));
    }
    let elapsed = start.elapsed();
    // Deep-cloning the growing cache takes tens of seconds; this margin tolerates loaded hosts.
    assert!(elapsed < Duration::from_secs(5), "took {elapsed:?}");
    transcript
        .event_until(|event| {
            matches!(
                event,
                Event::StateLimitReached {
                    limit: StateLimit::BouncerNetworks
                }
            )
        })
        .inspect(|session| assert_eq!(session.state().bouncer_networks().count(), 10_000))
        .ignore_rest();
}
