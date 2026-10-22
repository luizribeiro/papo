//! Transcript tests for upstream bouncer network management.
#![cfg(feature = "unstable")]

mod support;

use std::collections::BTreeMap;

use papo_core::{Config, Event, Feature, NetworkConfig, NetworkId, NetworkState, StateLimit};
use papo_proto::Casemapping;
use support::{Transcript, assert_session_debug_redacts, queue_debug_marker};

#[test]
fn network_ids_have_the_identifier_value_shape() {
    let owned = "1".to_owned();
    let from_borrowed = NetworkId::from(&owned);
    assert!(PartialEq::<str>::eq(&from_borrowed, "1"));
    assert!(!PartialEq::<str>::eq(&from_borrowed, "2"));
    assert_eq!(from_borrowed, "1");
    assert_ne!(from_borrowed, "2");
    assert_eq!(AsRef::<str>::as_ref(&from_borrowed), "1");
    assert_eq!(from_borrowed.to_string(), "1");
    assert_eq!(NetworkId::from(&from_borrowed), from_borrowed);
    assert_eq!(NetworkId::from(owned).into_string(), "1");

    let networks = BTreeMap::from([(NetworkId::from("1"), "irc.libera.chat")]);
    assert_eq!(networks.get("1"), Some(&"irc.libera.chat"));
    assert_eq!(networks.get("2"), None);
}

fn registered() -> Transcript {
    Transcript::new(Config::default().enable(Feature::Bouncer).wanted_caps([
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
    let mut config = Config::default()
        .enable(Feature::Bouncer)
        .wanted_caps(["soju.im/bouncer-networks"]);
    config.bouncer_network = Some(NetworkId::from("42"));
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
        .nickname(Casemapping::default().nick("alice"));
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
        .event(|event| matches!(event, Event::BouncerNetworkChanged { network: Some(network), .. }
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
                .bouncer_change(std::time::Instant::now(), &NetworkId::from("43"), &config)
                .unwrap();
        })
        .client(r"@label=2 BOUNCER CHANGENETWORK 43 host=irc.libera.chat;name=Libera\sChat;port=6697;tls=1;nickname=alice")
        .server("@label=2 BOUNCER CHANGENETWORK 43")
        .event(|event| matches!(event, Event::Response(response) if response.lines.len() == 1))
        .update(|session| {
            session
                .bouncer_remove(std::time::Instant::now(), &NetworkId::from("43"))
                .unwrap();
        })
        .client("@label=3 BOUNCER DELNETWORK 43")
        .server("@label=3 BOUNCER DELNETWORK 43")
        .event(|event| matches!(event, Event::Response(response) if response.lines.len() == 1))
        .no_event();
}

#[test]
fn redacts_a_queued_bouncer_network_password() {
    let config = NetworkConfig::new("irc.example.org").password("unique-bouncer-password-9241");
    registered()
        .update(|session| {
            session
                .bouncer_add(std::time::Instant::now(), &config)
                .unwrap();
            queue_debug_marker(session);
        })
        .inspect(|session| {
            assert_session_debug_redacts(session, "unique-bouncer-password-9241");
        })
        .client(
            "@label=0 BOUNCER ADDNETWORK host=irc.example.org;pass=unique-bouncer-password-9241",
        )
        .client("PRIVMSG #papo :ordinary debug marker");
}

#[test]
fn merges_bounded_notification_batches_and_deletes_networks() {
    registered()
        .server("BATCH +notify soju.im/bouncer-networks")
        .server(r"@batch=notify BOUNCER NETWORK 42 name=My\sNetwork;host=irc.example.org;state=connecting")
        .server("BATCH -notify")
        .event(|event| matches!(event, Event::BouncerNetworkChanged { network: Some(network), .. }
            if network.host.as_deref() == Some("irc.example.org")))
        .server("BOUNCER NETWORK 42 state=connected;host")
        .event(|event| matches!(event, Event::BouncerNetworkChanged { network: Some(network), .. }
            if network.host.is_none()))
        .inspect(|session| {
            let network = session.state().bouncer_networks().next().unwrap();
            assert_eq!(network.name.as_deref(), Some("My Network"));
        })
        .server("BOUNCER NETWORK 42 *")
        .event(|event| matches!(event, Event::BouncerNetworkChanged { id, network: None, .. }
            if id == "42"))
        .inspect(|session| assert_eq!(session.state().bouncer_networks().count(), 0))
        .no_event();
}

#[test]
fn parses_each_network_attribute_and_rejects_nearby_commands() {
    registered()
        .server(r"BOUNCER OTHER 42 name=Wrong\sSubcommand")
        .event(|event| {
            matches!(event, Event::Line(message) if message.as_ref().command_is("BOUNCER"))
        })
        .server(r"NOTICE NETWORK 42 name=Wrong\sCommand")
        .event(|event| {
            matches!(event, Event::Line(message) if message.as_ref().command_is("NOTICE"))
        })
        .server(
            r"BOUNCER NETWORK network-42 name=Example\sNet;state=connecting;host=irc.example.org;port=6697;tls=0;nickname=alice;username=alice_;realname=Alice\sExample;error=Connection\srefused;vendor.example/region=us-east",
        )
        .event(|event| {
            matches!(
                event,
                Event::BouncerNetworkChanged {
                    id,
                    network: Some(network),
                ..
                } if id == "network-42"
                    && network.name.as_deref() == Some("Example Net")
                    && network.state == Some(NetworkState::Connecting)
                    && network.host.as_deref() == Some("irc.example.org")
                    && network.port == Some(6697)
                    && network.tls == Some(false)
                    && network.nickname.as_deref() == Some("alice")
                    && network.nickname.as_ref().is_some_and(|nick| nick.casemapping() == Casemapping::Rfc1459)
                    && network.username.as_deref() == Some("alice_")
                    && network.realname.as_deref() == Some("Alice Example")
                    && network.error.as_deref() == Some("Connection refused")
                    && network.other.get("vendor.example/region").map(String::as_str)
                        == Some("us-east")
            )
        })
        .server("BOUNCER NETWORK network-42 state=disconnected")
        .event(|event| {
            matches!(
                event,
                Event::BouncerNetworkChanged {
                    network: Some(network),
                    ..
                } if network.state == Some(NetworkState::Disconnected)
            )
        })
        .no_event();
}

#[test]
fn refolds_cached_network_nicknames_when_casemapping_changes() {
    registered()
        .server("BOUNCER NETWORK 42 nickname=Alice[bot]")
        .event(|event| matches!(event, Event::BouncerNetworkChanged { .. }))
        .server(":ergo.test 005 papo CASEMAPPING=ascii :are supported")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .inspect(|session| {
            let nick = session
                .state()
                .bouncer_networks()
                .next()
                .unwrap()
                .nickname
                .as_ref()
                .unwrap();
            assert_eq!(nick.casemapping(), Casemapping::Ascii);
            assert_ne!(nick, "alice{bot}");
        })
        .ignore_rest();
}

#[test]
fn bounds_the_server_driven_network_cache() {
    let mut transcript = registered();
    for index in 0..10_001 {
        transcript = transcript.server(&format!(
            "BOUNCER NETWORK {index} host=irc{index}.example.org;state=disconnected"
        ));
    }
    transcript
        .event_until(|event| {
            matches!(
                event,
                Event::StateLimitReached {
                    limit: StateLimit::BouncerNetworks,
                    ..
                }
            )
        })
        .inspect(|session| assert_eq!(session.state().bouncer_networks().count(), 10_000))
        .ignore_rest();
}
