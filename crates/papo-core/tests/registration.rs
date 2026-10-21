//! Transcript and deadline tests for connection registration.

mod support;

use std::time::Duration;

use papo_core::{BatchAbandonReason, CloseKind, Config, Event, Secret, Session};
use papo_proto::Isupport;
use support::Transcript;

#[test]
fn sends_registration_in_protocol_order() {
    let config = Config::default()
        .nick("emily")
        .username("em")
        .realname("Emily Example")
        .password("correct horse battery staple");
    Transcript::new(config)
        .connect()
        .client("CAP LS 302")
        .client("PASS :correct horse battery staple")
        .client("NICK emily")
        .client("USER em 0 * :Emily Example")
        .no_client();
}

#[test]
fn rejects_a_byte_server_password() {
    Transcript::new(Config::default().password(Secret::from_bytes([0xff])))
        .connect()
        .client("CAP LS 302")
        .event(|event| matches!(event, Event::Close { kind: CloseKind::Other, reason, .. } if reason.contains("server password")))
        .no_client();
}

#[test]
fn closes_on_an_invalid_registration_message() {
    Transcript::new(Config::default().nick("papo\rOPER root password"))
        .connect()
        .client("CAP LS 302")
        .event(|event| matches!(event, Event::Close { kind: CloseKind::Other, reason, .. } if reason.contains("invalid outbound message")))
        .no_client();
}

#[test]
fn welcome_adopts_the_server_nick_and_cancels_the_timeout() {
    Transcript::new(Config::default().nick("oldnick"))
        .connect()
        .client("CAP LS 302")
        .client("NICK oldnick")
        .client("USER papo 0 * papo")
        .advance(Duration::from_secs(1))
        .welcome(":irc.example 001 account :Welcome to the network")
        .inspect(|session| assert_eq!(session.nick(), "account"))
        .deadline(Some(Duration::from_secs(10)));
}

#[test]
fn welcome_timeout_is_the_first_deadline_and_closes_once() {
    let timeout = Duration::from_secs(23);
    Transcript::new(Config::default().welcome_timeout(timeout))
        .connect_default()
        .deadline(Some(timeout))
        .advance(timeout.checked_sub(Duration::from_nanos(1)).unwrap())
        .no_event()
        .deadline(Some(Duration::from_nanos(1)))
        .advance(Duration::from_nanos(1))
        .event(|event| matches!(event, Event::Close { kind: CloseKind::WelcomeTimeout, reason, .. } if reason.contains("before 001")))
        .deadline(None)
        .advance(Duration::ZERO)
        .no_event();
}

#[test]
fn tries_each_nick_candidate_then_reports_exhaustion() {
    Transcript::new(Config::default().nick("emily").alt_nicks([
        "emily-away",
        "emily-work",
        "emily-backup",
        "emily-mobile",
    ]))
    .connect()
    .client("CAP LS 302")
    .client("NICK emily")
    .client("USER papo 0 * papo")
    .server(":ergo.test 005 * NICKLEN=1 :are supported by this server")
    .server(":ergo.test 433 * emily :Nickname is already in use")
    .client("NICK emily-away")
    .server(":ergo.test 437 * emily-away :Nick/channel is temporarily unavailable")
    .client("NICK emily-work")
    .server(":ergo.test 432 * emily-work :Erroneous nickname")
    .client("NICK emily-backup")
    .server(":ergo.test 436 * emily-backup :Nickname collision")
    .client("NICK emily-mobile")
    .server(":ergo.test 400 * NICK :Nickname is already in use")
    .client("NICK _")
    .server(":ergo.test 400 * NICK :Nickname is already in use")
    .event(|event| {
        matches!(event, Event::Close { kind: CloseKind::NickCandidatesExhausted, reason, .. }
            if reason == "nick candidates exhausted")
    })
    .no_client();
}

#[test]
fn unrelated_unknown_error_does_not_consume_a_nick_candidate() {
    Transcript::new(Config::default().alt_nicks(["papo-away"]))
        .connect_default()
        .server(":ergo.test 400 * PRIVMSG :Invalid target")
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("400")))
        .no_client()
        .server(":ergo.test 433 * papo :Nickname is already in use")
        .client("NICK papo-away");
}

#[test]
fn terminal_registration_numerics_close_with_their_wire_diagnostic() {
    for (numeric, text, expected) in [
        (
            "463",
            "No more connections allowed in your connection class",
            CloseKind::ConnectionRestricted,
        ),
        ("464", "Password incorrect", CloseKind::PasswordIncorrect),
        ("465", "You are banned from this server", CloseKind::Banned),
    ] {
        Transcript::new(Config::default())
            .connect_default()
            .server(&format!(":ergo.test {numeric} papo :{text}"))
            .event(|event| {
                matches!(event, Event::Close { kind, reason, .. } if *kind == expected && reason == text)
            })
            .no_client();
    }
}

#[test]
fn live_server_ban_closes_an_established_connection() {
    Transcript::new(Config::default())
        .connect_default()
        .welcome(":solanum.example 001 papo :Welcome to Solanum")
        .server(":solanum.example 422 papo :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
        .server(":solanum.example 465 papo :You are banned from this server")
        .event(|event| {
            matches!(event, Event::Close { kind: CloseKind::Banned, reason, .. }
                if reason == "You are banned from this server")
        })
        .no_client();
}

#[test]
fn generated_nicks_honor_an_early_nicklen_advertisement() {
    Transcript::new(Config::default().nick("long-client-nickname"))
        .connect()
        .client("CAP LS 302")
        .client("NICK long-client-nickname")
        .client("USER papo 0 * papo")
        .server(":ergo.test 005 * NICKLEN=12 :are supported by this server")
        .server(":ergo.test 433 * nick :Nickname is already in use")
        .client("NICK long-client_");
}

#[test]
fn requests_and_applies_extended_isupport_before_welcome() {
    Transcript::new(
        Config::default()
            .nick("long-client-nickname")
            .wanted_caps(["batch", "draft/extended-isupport"]),
    )
    .connect()
    .client("CAP LS 302")
    .client("NICK long-client-nickname")
    .client("USER papo 0 * papo")
    .server("CAP * LS :batch draft/extended-isupport")
    .client("CAP REQ :batch draft/extended-isupport")
    .server("CAP * ACK :batch draft/extended-isupport")
    .client("ISUPPORT")
    .client("CAP END")
    .server(":ergo.test BATCH +isupport draft/isupport")
    .server("@batch=isupport :ergo.test 005 * CASEMAPPING=ascii NICKLEN=12 :are supported")
    .inspect(|session| assert_eq!(session.isupport().nicklen(), None))
    .server("@batch=isupport :ergo.test 005 * NETWORK=ErgoTest :are supported")
    .server(":ergo.test BATCH -isupport")
    .event(|event| matches!(event, Event::CasemappingChanged { .. }))
    .inspect(|session| {
        assert_eq!(session.isupport().nicklen(), Some(12));
        assert_eq!(session.isupport().network(), Some("ErgoTest"));
    })
    .server(":ergo.test 433 * long-client-nickname :Nickname is already in use")
    .client("NICK long-client_")
    .welcome(":ergo.test 001 long-client_ :Welcome to Ergo")
    .server(":ergo.test 422 long-client_ :MOTD File is missing")
    .event(|event| {
        matches!(event, Event::Connected(welcome)
        if welcome.isupport.network() == Some("ErgoTest"))
    });
}

#[test]
fn incomplete_isupport_batch_does_not_change_state() {
    Transcript::new(
        Config::default()
            .welcome_timeout(Duration::from_secs(300))
            .ping_interval(Duration::from_secs(300))
            .ping_timeout(Duration::from_secs(300)),
    )
    .connect_default()
    .server(":ergo.test BATCH +isupport draft/isupport")
    .server("@batch=isupport :ergo.test 005 * NICKLEN=12 :are supported")
    .advance(Duration::from_secs(120))
    .event(|event| matches!(event, Event::Line(_)))
    .event(|event| {
        matches!(event, Event::BatchAbandoned {
            batch_type,
            reason: BatchAbandonReason::Timeout,
            ..
        } if batch_type == "draft/isupport")
    })
    .inspect(|session| assert_eq!(session.isupport().nicklen(), None));
}

#[test]
fn isupport_batches_change_generation_only_when_state_changes() {
    let now = std::time::Instant::now();
    let mut session = Session::new(Config::default());
    let generation = session.state_generation();

    for reference in ["first", "same"] {
        session.handle_line(now, format!("BATCH +{reference} draft/isupport").as_bytes());
        session.handle_line(
            now,
            format!("@batch={reference} :ergo.test 005 * CASEMAPPING=ascii :are supported")
                .as_bytes(),
        );
        session.handle_line(now, format!("BATCH -{reference}").as_bytes());
    }

    assert_eq!(
        session.isupport().casemapping(),
        papo_proto::Casemapping::Ascii
    );
    assert_eq!(session.state_generation(), generation + 1);
}

#[test]
fn generated_nicks_handle_the_smallest_advertised_limits() {
    for (limit, expected) in [(1, "_"), (2, "p_"), (3, "pa_")] {
        Transcript::new(Config::default())
            .connect_default()
            .server(&format!(
                ":ergo.test 005 * NICKLEN={limit} :are supported by this server"
            ))
            .server(":ergo.test 433 * papo :Nickname is already in use")
            .client(&format!("NICK {expected}"));
    }
}

#[test]
fn completes_after_motd_with_capabilities_and_isupport() {
    Transcript::new(
        Config::default()
            .nick("requested")
            .wanted_caps(["server-time"]),
    )
    .connect()
    .client("CAP LS 302")
    .client("NICK requested")
    .client("USER papo 0 * papo")
    .server("CAP * LS :server-time=iso8601")
    .client("CAP REQ :server-time")
    .server("CAP * ACK :server-time")
    .client("CAP END")
    .welcome(":ergo.test 001 account :Welcome to Ergo")
    .server(":ergo.test 005 account CASEMAPPING=ascii NICKLEN=32 :are supported by this server")
    .event(|event| matches!(event, Event::CasemappingChanged { .. }))
    .server(":ergo.test 005 account NETWORK=Ergo UTF8ONLY :are supported by this server")
    .server(":ergo.test 376 account :End of MOTD")
    .inspect(|session| assert!(session.is_connected()))
    .event(|event| {
        matches!(
            event,
            Event::Connected(welcome)
                if welcome.nick == "account"
                    && welcome.nick.casemapping() == papo_proto::Casemapping::Ascii
                    && welcome.caps.get("server-time") == Some(&Some("iso8601".to_owned()))
                    && welcome.isupport.network() == Some("Ergo")
                    && welcome.isupport.utf8only()
        )
    });
}

#[test]
fn emits_the_welcome_burst_in_wire_order_before_registration() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 001 emily :Welcome to the Ergo test network")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("001")))
        .server(":ergo.test 002 emily :Your host is ergo.test, running version ergo-2.16.0")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("002")))
        .server(":ergo.test 003 emily :This server was created Wed Oct 15 2025")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("003")))
        .server(":ergo.test 004 emily ergo.test ergo-2.16.0 BioZ beI,k,l,imnst")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("004")))
        .server(":ergo.test 005 emily NETWORK=ErgoTest UTF8ONLY :are supported by this server")
        .server(":ergo.test 251 emily :There are 4 users and 0 invisible on 1 server")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("251")))
        .server(":ergo.test 375 emily :- ergo.test Message of the Day -")
        .server(":ergo.test 372 emily :- Be kind to one another.")
        .no_event()
        .server(":ergo.test 376 emily :End of MOTD")
        .event(|event| {
            matches!(event, Event::Motd { lines, .. }
                if lines == &["- Be kind to one another."])
        })
        .event(|event| {
            matches!(event, Event::Connected(welcome)
                if welcome.text == "Welcome to the Ergo test network")
        })
        .no_event();
}

#[test]
fn caps_an_unfinished_motd_stream() {
    let transcript = Transcript::new(Config::default())
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 375 papo :- ergo.test Message of the Day -");
    let transcript = (0..1_001).fold(transcript, |transcript, _| {
        transcript.server(":ergo.test 372 papo :- This server is managed by Example Networks")
    });
    transcript
        .no_event()
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Motd { lines, .. } if lines.len() == 1_000))
        .event(|event| matches!(event, Event::Connected(_)))
        .no_event();
}

#[test]
fn emits_an_empty_completed_motd() {
    Transcript::new(Config::default())
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 375 papo :- ergo.test Message of the Day -")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Motd { lines, .. } if lines.is_empty()))
        .event(|event| matches!(event, Event::Connected(_)))
        .no_event();
}

#[test]
fn no_motd_numeric_completes_registration() {
    Transcript::new(Config::default())
        .connect_default()
        .welcome(":solanum.example 001 papo :Welcome")
        .server(":solanum.example 422 papo :MOTD File is missing")
        .event(|event| {
            matches!(
                event,
                Event::Connected(welcome)
                    if welcome.nick == "papo" && welcome.caps.is_empty()
            )
        })
        .inspect(|session| assert!(session.is_connected()))
        .no_event();
}

#[test]
fn completes_ten_seconds_after_welcome_when_motd_never_finishes() {
    Transcript::new(Config::default())
        .connect_default()
        .welcome(":soju.example 001 papo :Welcome")
        .server(":soju.example 375 papo :- soju.example Message of the Day -")
        .server(":soju.example 372 papo :- This MOTD never finishes")
        .deadline(Some(Duration::from_secs(10)))
        .advance(Duration::from_secs(9))
        .inspect(|session| assert!(!session.is_connected()))
        .no_event()
        .deadline(Some(Duration::from_secs(1)))
        .advance(Duration::from_secs(1))
        .inspect(|session| assert!(session.is_connected()))
        .event(|event| matches!(event, Event::Connected(_)))
        .no_event()
        .deadline(Some(Duration::from_secs(50)));
}

#[test]
fn reconnect_resets_all_registration_state() {
    Transcript::new(
        Config::default()
            .alt_nicks(["papo-away"])
            .wanted_caps(["server-time"]),
    )
    .connect_default()
    .server("CAP * LS :server-time")
    .client("CAP REQ :server-time")
    .server("CAP * ACK :server-time")
    .client("CAP END")
    .server(":ergo.test 433 * papo :Nickname is already in use")
    .client("NICK papo-away")
    .server(":ergo.test 005 * NICKLEN=32 NETWORK=ErgoTest :are supported by this server")
    .welcome(":ergo.test 001 papo-away :Welcome to Ergo")
    .server(":ergo.test 422 papo-away :MOTD File is missing")
    .event(|event| matches!(event, Event::Connected(_)))
    .inspect(|session| {
        assert!(session.is_connected());
        assert!(session.caps().contains_key("server-time"));
        assert_eq!(session.isupport().network(), Some("ErgoTest"));
        assert_eq!(session.nick(), "papo-away");
    })
    .disconnect()
    .inspect(|session| {
        assert!(!session.is_connected());
        assert_eq!(session.poll_timeout(), None);
    })
    .connect_default()
    .inspect(|session| {
        assert!(session.caps().is_empty());
        assert_eq!(session.isupport(), &Isupport::default());
        assert_eq!(session.nick(), "papo");
        assert!(session.poll_timeout().is_some());
    });
}
