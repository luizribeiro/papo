//! Transcript tests for trusted gateway registration with `WEBIRC`.

mod support;

use papo_core::{CloseKind, Config, Event, Secret, Session, WebIrc};
use support::{Transcript, assert_session_debug_redacts, queue_debug_marker};

fn webirc(client_hostname: Option<&str>, client_ip: &str) -> WebIrc {
    WebIrc {
        password: Secret::new("gateway-password"),
        gateway: "papo-gateway".into(),
        client_ip: client_ip.parse().unwrap(),
        client_hostname: client_hostname.map(str::to_owned),
        options: Vec::new(),
    }
}

#[test]
fn sends_ipv4_webirc_before_cap_and_reuses_an_unknown_hostname() {
    Transcript::new(Config::default().webirc(webirc(None, "192.0.2.42")))
        .connect()
        .client("WEBIRC gateway-password papo-gateway 192.0.2.42 192.0.2.42")
        .client("CAP LS 302")
        .client("NICK papo")
        .client("USER papo 0 * papo")
        .no_client();
}

#[test]
fn closes_on_an_invalid_webirc_message() {
    let mut gateway = webirc(None, "192.0.2.42");
    gateway.gateway = "papo\rgateway".into();
    Transcript::new(Config::default().webirc(gateway))
        .connect()
        .event(|event| matches!(event, Event::Close { kind: CloseKind::Other, reason, .. } if reason.contains("invalid WEBIRC")))
        .no_client();
}

#[test]
fn prefixes_leading_colon_ipv6_addresses() {
    Transcript::new(Config::default().webirc(webirc(None, "::ffff:192.0.2.42")))
        .connect()
        .client("WEBIRC gateway-password papo-gateway 0::ffff:192.0.2.42 0::ffff:192.0.2.42")
        .client("CAP LS 302")
        .client("NICK papo")
        .client("USER papo 0 * papo")
        .no_client();
}

#[test]
fn preserves_option_order_and_escapes_values() {
    let mut gateway = webirc(Some("client.example"), "192.0.2.42");
    gateway.options = vec![
        ("secure".into(), None),
        ("remote-port".into(), Some("6697".into())),
        ("certfp-sha-256".into(), Some("ab cd;ef\\gh".into())),
    ];
    Transcript::new(Config::default().webirc(gateway))
        .connect()
        .client(
            r"WEBIRC gateway-password papo-gateway client.example 192.0.2.42 :secure remote-port=6697 certfp-sha-256=ab\scd\:ef\\gh",
        )
        .client("CAP LS 302")
        .client("NICK papo")
        .client("USER papo 0 * papo")
        .no_client();
}

#[test]
fn redacts_the_webirc_password_from_configuration_and_session_debug() {
    let password = "unique-webirc-gateway-password";
    let config = Config::default()
        .webirc(WebIrc {
            password: Secret::new(password),
            ..webirc(None, "192.0.2.42")
        })
        .allow_insecure_auth(true);
    assert!(!format!("{config:?}").contains(password));
    let mut session = Session::new(config);
    session.handle_connected(std::time::Instant::now());
    queue_debug_marker(&mut session);
    assert_session_debug_redacts(&session, password);
}

#[test]
fn closes_without_more_protocol_work_when_webirc_is_rejected() {
    Transcript::new(Config::default().webirc(webirc(None, "192.0.2.42")))
        .connect()
        .client("WEBIRC gateway-password papo-gateway 192.0.2.42 192.0.2.42")
        .client("CAP LS 302")
        .client("NICK papo")
        .client("USER papo 0 * papo")
        .server("ERROR :WEBIRC command is not usable from your address or incorrect password given")
        .event(|event| {
            matches!(event, Event::Close { kind: CloseKind::WebIrcRejected, reason, .. }
                if reason == "WEBIRC command is not usable from your address or incorrect password given")
        })
        .server("PING :ergo.test")
        .no_client();
}

#[test]
fn unrelated_first_error_is_a_plain_server_refusal() {
    Transcript::new(Config::default().webirc(webirc(None, "192.0.2.42")))
        .connect()
        .client("WEBIRC gateway-password papo-gateway 192.0.2.42 192.0.2.42")
        .client("CAP LS 302")
        .client("NICK papo")
        .client("USER papo 0 * papo")
        .server("ERROR :Too many clients from your network")
        .event(|event| {
            matches!(event, Event::Close { kind: CloseKind::ServerError, reason, .. }
                if reason == "Too many clients from your network")
        });
}
