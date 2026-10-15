//! Transcript tests for `IRCv3` capability negotiation.

mod support;

use std::time::{Duration, SystemTime};

use papo_core::sts::{ConnectionSecurity, StsAction};
use papo_core::{Config, Event, Session};
use support::Transcript;

#[test]
fn collects_values_selects_aliases_and_denies_the_trap() {
    let config =
        Config::default().wanted_caps(["server-time", "no-implicit-names", "ergo.chat/nope"]);
    Transcript::new(config)
        .connect()
        .client("CAP LS 302")
        .client("NICK papo")
        .client("USER papo 0 * papo")
        .server("CAP * LS * :server-time=old sasl=PLAIN,EXTERNAL")
        .no_client()
        .server("CAP * LS :server-time=latest draft/no-implicit-names")
        .client("CAP REQ :server-time draft/no-implicit-names")
        .no_client()
        .server("CAP * ACK :server-time draft/no-implicit-names")
        .client("CAP END");

    let mut session =
        Session::new(Config::default().wanted_caps(["server-time", "no-implicit-names"]));
    session.handle_connected(std::time::Instant::now());
    session.handle_line(
        std::time::Instant::now(),
        b"CAP * LS :server-time=latest draft/no-implicit-names",
    );
    session.handle_line(
        std::time::Instant::now(),
        b"CAP * ACK :server-time draft/no-implicit-names",
    );
    assert_eq!(
        session.caps().get("server-time"),
        Some(&Some("latest".to_owned()))
    );
    assert_eq!(session.caps().get("draft/no-implicit-names"), Some(&None));
    assert!(session.caps().contains_key("cap-notify"));
}

#[test]
fn ends_immediately_when_no_advertised_cap_is_wanted() {
    Transcript::new(Config::default().wanted_caps(Vec::<String>::new()))
        .connect_default()
        .server("CAP * LS :")
        .client("CAP END");

    Transcript::new(Config::default().wanted_caps(["ergo.chat/nope"]))
        .connect_default()
        .server("CAP * LS :ergo.chat/nope")
        .client("CAP END");
}

#[test]
fn capability_alias_resolution_stays_in_its_namespace() {
    Transcript::new(Config::default().wanted_caps(["batch"]))
        .connect_default()
        .server("CAP * LS :no-implicit-names")
        .client("CAP END");

    Transcript::new(Config::default().wanted_caps(["+draft/reply"]))
        .connect_default()
        .server("CAP * LS :+reply")
        .client("CAP END");
}

#[test]
fn drops_a_rejected_request_and_ends_negotiation() {
    Transcript::new(Config::default().wanted_caps(["batch", "server-time"]))
        .connect_default()
        .server("CAP * LS :batch server-time")
        .client("CAP REQ :batch server-time")
        .server("CAP * NAK :batch server-time")
        .client("CAP END");
}

#[test]
fn splits_requests_at_510_bytes_and_resolves_each_atomically() {
    let first = format!("example.com/{}", "a".repeat(238));
    let second = format!("example.net/{}", "b".repeat(238));
    let third = "batch".to_owned();
    let config = Config::default().wanted_caps([first.clone(), second.clone(), third.clone()]);
    Transcript::new(config)
        .connect_default()
        .server(&format!("CAP * LS * :{first}"))
        .server(&format!("CAP * LS * :{second}"))
        .server(&format!("CAP * LS :{third}"))
        .client(&format!("CAP REQ :{first} {second}"))
        .client("CAP REQ :batch")
        .server("CAP * NAK :request rejected")
        .no_client()
        .server("CAP * ACK * :bat")
        .no_client()
        .server("CAP * ACK :ch")
        .client("CAP END")
        .no_client();

    let first_request = format!("CAP REQ :{first} {second}");
    assert_eq!(first_request.len(), 510);
}

#[test]
fn keeps_510_bytes_together_and_splits_511_bytes() {
    let first = format!("example.com/{}", "a".repeat(238));
    let fits = format!("example.net/{}", "b".repeat(238));
    let exact = format!("CAP REQ :{first} {fits}");
    assert_eq!(exact.len(), 510);
    Transcript::new(Config::default().wanted_caps([&first, &fits]))
        .connect_default()
        .server(&format!("CAP * LS * :{first}"))
        .server(&format!("CAP * LS :{fits}"))
        .client(&exact)
        .no_client()
        .server("CAP * ACK :accepted")
        .client("CAP END");

    let splits = format!("example.net/{}", "b".repeat(239));
    let too_long = format!("CAP REQ :{first} {splits}");
    assert_eq!(too_long.len(), 511);
    Transcript::new(Config::default().wanted_caps([&first, &splits]))
        .connect_default()
        .server(&format!("CAP * LS * :{first}"))
        .server(&format!("CAP * LS :{splits}"))
        .client(&format!("CAP REQ :{first}"))
        .client(&format!("CAP REQ :{splits}"))
        .server("CAP * ACK :first accepted")
        .no_client()
        .server("CAP * ACK :second accepted")
        .client("CAP END");
}

#[test]
fn ignores_cap_errors_and_waits_for_welcome() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":legacy.example 421 papo CAP :Unknown command")
        .no_client()
        .server(":legacy.example 001 papo :Welcome")
        .no_client();

    Transcript::new(Config::default())
        .connect_default()
        .server(":broken.example 410 papo LS :Invalid CAP command")
        .no_client()
        .server(":broken.example 001 papo :Welcome")
        .no_client();
}

#[test]
fn surfaces_unknown_command_for_commands_other_than_cap() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":legacy.example 421 papo WHOIS :Unknown command")
        .event(|event| {
            matches!(
                event,
                Event::Message(message)
                    if message.as_ref().command_is("421")
                        && message.as_ref().param(1) == Some("WHOIS")
            )
        });
}

#[test]
fn enables_new_wanted_caps_and_updates_enabled_values() {
    Transcript::new(Config::default().wanted_caps(["batch", "server-time"]))
        .connect_default()
        .server("CAP * LS :server-time=old")
        .client("CAP REQ :server-time")
        .server("CAP * ACK :server-time")
        .client("CAP END")
        .server("CAP papo NEW :batch server-time=latest")
        .event(|event| {
            matches!(event, Event::CapsChanged { added, removed }
            if added.get("server-time") == Some(&Some("latest".to_owned()))
                && removed.is_empty())
        })
        .client("CAP REQ :batch")
        .server("CAP papo ACK :batch")
        .event(|event| {
            matches!(event, Event::CapsChanged { added, removed }
            if added.get("batch") == Some(&None) && removed.is_empty())
        })
        .inspect(|session| {
            assert_eq!(
                session.caps().get("server-time"),
                Some(&Some("latest".to_owned()))
            );
            assert!(session.caps().contains_key("batch"));
        });
}

#[test]
fn drops_deleted_caps_without_requesting_disable_and_can_enable_them_again() {
    Transcript::new(Config::default().wanted_caps(["server-time"]))
        .connect_default()
        .server("CAP * LS :server-time=2026")
        .client("CAP REQ :server-time")
        .server("CAP * ACK :server-time")
        .client("CAP END")
        .server("CAP papo DEL :server-time")
        .event(|event| {
            matches!(event, Event::CapsChanged { added, removed }
            if added.is_empty()
                && removed.get("server-time") == Some(&Some("2026".to_owned())))
        })
        .inspect(|session| assert!(!session.caps().contains_key("server-time")))
        .no_client()
        .server("CAP papo NEW :server-time=2027")
        .client("CAP REQ :server-time")
        .server("CAP papo ACK :server-time")
        .event(|event| {
            matches!(event, Event::CapsChanged { added, removed }
            if added.get("server-time") == Some(&Some("2027".to_owned()))
                && removed.is_empty())
        })
        .inspect(|session| assert!(session.caps().contains_key("server-time")));
}

#[test]
fn sts_on_plaintext_closes_for_a_tls_upgrade_without_more_output() {
    Transcript::new(
        Config::default()
            .host("irc.example.net")
            .wanted_caps(["sts", "batch"]),
    )
    .connect_default()
    .server("CAP * LS :sts=port=6697,duration=86400 batch")
    .event(|event| matches!(event, Event::StsUpgrade { port: 6697 }))
    .event(|event| matches!(event, Event::Close { reason } if reason == "sts upgrade"))
    .no_client();
}

#[test]
fn sts_on_tls_surfaces_persist_refresh_and_remove_actions() {
    let anchor = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    Transcript::new(
        Config::default()
            .connection_security(ConnectionSecurity::Secure)
            .host("IRC.Example.NET.")
            .sts_time(anchor)
            .wanted_caps(["sts"]),
    )
    .connect_default()
    .server("CAP * LS :sts=duration=3600,preload")
    .event(|event| {
        matches!(event, Event::StsPolicy(StsAction::Persist { host, expires_at })
        if host == "irc.example.net"
            && *expires_at == anchor + Duration::from_secs(3_600))
    })
    .client("CAP END")
    .server("CAP papo NEW :sts=duration=7200")
    .event(|event| {
        matches!(event, Event::StsPolicy(StsAction::Persist { expires_at, .. })
        if *expires_at == anchor + Duration::from_secs(7_200))
    })
    .server("CAP papo NEW :sts=duration=0")
    .event(|event| {
        matches!(event, Event::StsPolicy(StsAction::Remove { host })
        if host == "irc.example.net")
    })
    .server("CAP papo DEL :sts")
    .no_event()
    .no_client();
}
