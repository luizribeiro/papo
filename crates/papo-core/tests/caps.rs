//! Transcript tests for `IRCv3` capability negotiation.

mod support;

use std::time::Duration;

use papo_core::sts::{ConnectionSecurity, StsAction};
use papo_core::{CloseKind, Config, Connection, DEFAULT_WANTED_CAPS, Event, Session, cap::Cap};
#[cfg(feature = "unstable")]
use papo_core::{Feature, UnsupportedReason};
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
        .server("CAP * LS :server-time=latest no-implicit-names")
        .client("CAP REQ :server-time no-implicit-names")
        .no_client()
        .server("CAP * ACK :server-time no-implicit-names")
        .client("CAP END");

    let mut session =
        Session::new(Config::default().wanted_caps(["server-time", "no-implicit-names"]));
    session.handle_connected(std::time::Instant::now());
    session.handle_line(
        std::time::Instant::now(),
        b"CAP * LS :server-time=latest no-implicit-names",
    );
    session.handle_line(
        std::time::Instant::now(),
        b"CAP * ACK :server-time no-implicit-names",
    );
    assert_eq!(
        session.caps().get("server-time"),
        Some(&Some("latest".to_owned()))
    );
    assert_eq!(
        session.caps().get(&Cap::from("draft/no-implicit-names")),
        Some(&None)
    );
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

    Transcript::new(Config::default().wanted_caps(["draft/chathistory"]))
        .connect_default()
        .server("CAP * LS :chathistory")
        .client("CAP END");
}

#[test]
fn requests_event_playback_and_oper_tags_by_default() {
    Transcript::new(Config::default())
        .connect_default()
        .server("CAP * LS :draft/event-playback draft/oper-tag")
        .client("CAP REQ :draft/event-playback draft/oper-tag")
        .server("CAP * ACK :draft/event-playback draft/oper-tag")
        .client("CAP END")
        .server(
            "@draft/oper=network-admin :alice!a@localhost PRIVMSG #papo :Maintenance begins soon",
        )
        .event(|event| {
            matches!(event, Event::Line(message)
            if message.as_ref().tag("draft/oper").as_deref() == Some("network-admin"))
        });
}

#[test]
fn requests_echo_message_only_with_labeled_response() {
    Transcript::new(Config::default().wanted_caps(["echo-message", "labeled-response"]))
        .connect_default()
        .server("CAP * LS :echo-message")
        .client("CAP END");

    Transcript::new(Config::default().wanted_caps(["echo-message", "labeled-response"]))
        .connect_default()
        .server("CAP * LS :echo-message labeled-response")
        .client("CAP REQ :echo-message labeled-response")
        .server("CAP * ACK :echo-message labeled-response")
        .client("CAP END");

    Transcript::new(Config::default().wanted_caps(["echo-message"]))
        .connect_default()
        .server("CAP * LS :echo-message labeled-response")
        .client("CAP END");

    Transcript::new(Config::default().wanted_caps(["echo-message", "labeled-response"]))
        .connect_default()
        .server("CAP * LS :")
        .client("CAP END")
        .server("CAP papo NEW :echo-message")
        .no_client()
        .server("CAP papo NEW :labeled-response")
        .client("CAP REQ :echo-message labeled-response")
        .server("CAP papo ACK :echo-message labeled-response")
        .event(|event| {
            matches!(event, Event::CapsChanged { added, removed, .. }
                if added.contains_key("echo-message")
                    && added.contains_key("labeled-response")
                    && removed.is_empty())
        })
        .inspect(|session| {
            assert!(session.caps().contains_key("echo-message"));
            assert!(session.caps().contains_key("labeled-response"));
        });
}

#[test]
fn cap_new_requests_only_capabilities_newly_available_in_that_update() {
    let config = Config::default().wanted_caps([
        "server-time",
        "echo-message",
        "labeled-response",
        "ergo.chat/nope",
    ]);
    Transcript::new(config.clone())
        .connect_default()
        .server("CAP * LS :echo-message")
        .client("CAP END")
        .server("CAP papo NEW :server-time")
        .client("CAP REQ :server-time")
        .server("CAP papo NAK :server-time")
        .server("CAP papo NEW :ergo.chat/nope")
        .no_client();

    Transcript::new(config)
        .connect_default()
        .server("CAP * LS :echo-message labeled-response")
        .client("CAP REQ :echo-message labeled-response")
        .server("CAP * NAK :echo-message labeled-response")
        .client("CAP END")
        .server("CAP papo NEW :server-time")
        .client("CAP REQ :server-time")
        .server("CAP papo NAK :server-time")
        .server("CAP papo NEW :ergo.chat/nope")
        .no_client();
}

#[test]
fn defaults_cover_every_implemented_negotiable_capability() {
    assert_eq!(
        DEFAULT_WANTED_CAPS,
        [
            "cap-notify",
            "message-tags",
            "server-time",
            "batch",
            "draft/event-playback",
            "echo-message",
            "account-tag",
            "extended-join",
            "multi-prefix",
            "userhost-in-names",
            "chghost",
            "setname",
            "away-notify",
            "account-notify",
            "extended-monitor",
            "invite-notify",
            "standard-replies",
            "labeled-response",
            "draft/multiline",
            "draft/oper-tag",
            "draft/metadata-2",
            "draft/account-registration",
            "draft/chathistory",
            "draft/read-marker",
            "draft/channel-rename",
            "draft/message-redaction",
            "draft/pre-away",
            "no-implicit-names",
            "draft/whoami",
            "draft/extended-isupport",
        ]
    );
}

#[cfg(feature = "unstable")]
#[test]
fn unstable_capabilities_require_explicit_feature_selection() {
    let offered = "soju.im/bouncer-networks soju.im/bouncer-networks-notify draft/persistence draft/relaymsg draft/webpush";
    Transcript::new(Config::default())
        .connect_default()
        .server(&format!("CAP * LS :{offered}"))
        .client("CAP END")
        .inspect(|session| {
            assert_eq!(
                session.supports(Feature::Bouncer),
                Err(UnsupportedReason::DisabledByClient)
            );
        })
        .server("CAP papo NEW :draft/webpush")
        .no_client();

    Transcript::new(
        Config::default()
            .enable(Feature::Bouncer)
            .enable(Feature::Persistence)
            .enable(Feature::Relay)
            .enable(Feature::WebPush),
    )
    .connect_default()
    .server(&format!("CAP * LS :{offered}"))
    .client(&format!("CAP REQ :{offered}"))
    .server(&format!("CAP * ACK :{offered}"))
    .client("CAP END");
}

#[test]
fn naming_unstable_capabilities_does_not_opt_in_or_request_them_later() {
    let offered = "soju.im/bouncer-networks soju.im/bouncer-networks-notify draft/persistence draft/relaymsg draft/webpush";
    Transcript::new(Config::default().wanted_caps(offered.split_whitespace()))
        .connect_default()
        .server(&format!("CAP * LS :{offered}"))
        .client("CAP END");

    #[cfg(feature = "unstable")]
    Transcript::new(Config::default().enable(Feature::WebPush))
        .connect_default()
        .server("CAP * LS :")
        .client("CAP END")
        .server("CAP papo NEW :draft/webpush")
        .no_client();
}

#[test]
fn requests_only_published_aliases_in_the_capability_namespace() {
    Transcript::new(Config::default())
        .connect_default()
        .server("CAP * LS :chathistory")
        .client("CAP END");

    Transcript::new(Config::default().wanted_caps(["draft/chathistory"]))
        .connect_default()
        .server("CAP * LS :draft/chathistory")
        .client("CAP REQ :draft/chathistory")
        .server("CAP * ACK :draft/chathistory")
        .client("CAP END");

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
fn explicitly_disabled_caps_are_never_requested() {
    Transcript::new(
        Config::default()
            .without_caps(["chathistory", "server-time"])
            .wanted_caps(["draft/chathistory", "server-time", "batch"]),
    )
    .connect_default()
    .server("CAP * LS :chathistory server-time batch")
    .client("CAP REQ :batch")
    .server("CAP * ACK :batch")
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
        .welcome(":legacy.example 001 papo :Welcome")
        .no_client();

    Transcript::new(Config::default())
        .connect_default()
        .server(":broken.example 410 papo LS :Invalid CAP command")
        .no_client()
        .welcome(":broken.example 001 papo :Welcome")
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
                Event::Line(message)
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
            matches!(event, Event::CapsChanged { added, removed, .. }
            if added.get("server-time") == Some(&Some("latest".to_owned()))
                && removed.is_empty())
        })
        .client("CAP REQ :batch")
        .server("CAP papo ACK :batch")
        .event(|event| {
            matches!(event, Event::CapsChanged { added, removed, .. }
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
            matches!(event, Event::CapsChanged { added, removed, .. }
            if added.is_empty()
                && removed.get("server-time") == Some(&Some("2026".to_owned())))
        })
        .inspect(|session| assert!(!session.caps().contains_key("server-time")))
        .no_client()
        .server("CAP papo NEW :server-time=2027")
        .client("CAP REQ :server-time")
        .server("CAP papo ACK :server-time")
        .event(|event| {
            matches!(event, Event::CapsChanged { added, removed, .. }
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
    .event(|event| matches!(event, Event::StsUpgrade { port: 6697, .. }))
    .event(|event| matches!(event, Event::Close { kind: CloseKind::StsUpgrade, reason, .. } if reason == "sts upgrade"))
    .no_client();
}

#[test]
fn sts_on_tls_surfaces_persist_refresh_and_remove_actions() {
    Transcript::new(
        Config::default()
            .connection_security(ConnectionSecurity::Secure)
            .host("IRC.Example.NET.")
            .wanted_caps(["sts"]),
    )
    .connect_default()
    .server("CAP * LS :sts=duration=3600,preload")
    .event(|event| {
        matches!(event, Event::StsPolicy(StsAction::Persist { host, duration, .. })
        if host == "irc.example.net"
            && *duration == Duration::from_secs(3_600))
    })
    .client("CAP END")
    .server("CAP papo NEW :sts=duration=7200")
    .event(|event| {
        matches!(event, Event::StsPolicy(StsAction::Persist { duration, .. })
        if *duration == Duration::from_secs(7_200))
    })
    .server("CAP papo NEW :sts=duration=0")
    .event(|event| {
        matches!(event, Event::StsPolicy(StsAction::Remove { host, .. })
        if host == "irc.example.net")
    })
    .server("CAP papo DEL :sts")
    .no_event()
    .no_client();
}

#[test]
fn sts_upgrade_can_reconnect_the_same_session_securely() {
    Transcript::new(Config::default().wanted_caps(["sts"]))
        .connect_with(Connection {
            host: "irc.example.net".to_owned(),
            security: ConnectionSecurity::Plaintext,
        })
        .client("CAP LS 302")
        .client("NICK papo")
        .client("USER papo 0 * papo")
        .server("CAP * LS :sts=port=6697")
        .event(|event| matches!(event, Event::StsUpgrade { port: 6697, .. }))
        .event(|event| matches!(event, Event::Close { kind: CloseKind::StsUpgrade, reason, .. } if reason == "sts upgrade"))
        .disconnect()
        .connect_with(Connection {
            host: "IRC.Example.NET.".to_owned(),
            security: ConnectionSecurity::Secure,
        })
        .client("CAP LS 302")
        .client("NICK papo")
        .client("USER papo 0 * papo")
        .server("CAP * LS :sts=duration=86400")
        .event(|event| {
            matches!(event, Event::StsPolicy(StsAction::Persist { host, duration, .. })
                if host == "irc.example.net" && *duration == Duration::from_hours(24))
        })
        .client("CAP END");
}
