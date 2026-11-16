//! Transcript tests for session-owned ISUPPORT state.

mod support;

use papo_core::{Config, Event, Session, StateLimit};
use papo_proto::Advertised;
use support::Transcript;

#[test]
fn name_rules_require_an_explicit_casemapping_or_seed() {
    let now = std::time::Instant::now();
    let mut session = Session::new(Config::default());
    assert!(!session.__name_rules_known());

    session.handle_line(now, b":ergo.test 005 papo NETWORK=Ergo :are supported");
    assert!(!session.__name_rules_known());

    session.handle_line(now, b":ergo.test 005 papo CASEMAPPING=ascii :are supported");
    assert!(session.__name_rules_known());

    let seeded = Session::new(Config::default().casemapping(papo_proto::Casemapping::Ascii));
    assert!(seeded.__name_rules_known());
}

#[test]
fn applies_multiple_005_lines_and_enforces_utf8only() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test 005 papo CASEMAPPING=ascii NICKLEN=32 :are supported by this server")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .server(":ergo.test 005 papo NETWORK=Ergo UTF8ONLY :are supported by this server")
        .inspect(|session| {
            assert_eq!(session.isupport().nicklen(), Some(32));
            assert_eq!(session.isupport().network(), Some("Ergo"));
            assert_eq!(session.isupport().get("UTF8ONLY"), Advertised::Bare);
        })
        .server_bytes(b":ergo.test NOTICE papo :caf\xe9")
        .event(|event| event == &Event::__malformed(b":ergo.test NOTICE papo :caf\xe9".to_vec()));
}

#[test]
fn removing_utf8only_restores_the_configured_decoding_policy() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":soju.example 005 papo UTF8ONLY :are supported by this server")
        .server(":soju.example 005 papo -UTF8ONLY :are supported by this server")
        .server_bytes(b":soju.example NOTICE papo :caf\xe9")
        .event(|event| matches!(event, Event::Line(_)));
}

#[test]
fn warns_once_per_connection_when_rfc8265_cannot_be_applied() {
    let now = std::time::Instant::now();
    let mut session = Session::new(Config::default());
    for _ in 0..2 {
        session.handle_connected(now);
        while session.poll_event().is_some() {}
        for line in [
            ":ergo.test 005 papo CASEMAPPING=ascii UTF8MAPPING=rfc8265 :are supported",
            ":ergo.test 005 papo UTF8MAPPING=rfc8265 :are supported",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        let warnings = std::iter::from_fn(|| session.poll_event())
            .filter(|event| matches!(event, Event::Utf8CasemappingUnsupported))
            .count();
        let supports_rfc8265 = session.isupport().casemapping().as_str() == "rfc7613";
        assert_eq!(warnings, usize::from(!supports_rfc8265));
        session.handle_disconnected(now);
    }
}

#[test]
fn remembers_pending_utf8_warning_across_isupport_lines() {
    let now = std::time::Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_connected(now);
    while session.poll_event().is_some() {}
    session.handle_line(now, b":ergo.test 005 papo CASEMAPPING=ascii :are supported");
    while session.poll_event().is_some() {}
    session.handle_line(
        now,
        b":ergo.test 005 papo UTF8MAPPING=rfc8265 :are supported",
    );
    let warnings = std::iter::from_fn(|| session.poll_event())
        .filter(|event| matches!(event, Event::Utf8CasemappingUnsupported))
        .count();
    let supports_rfc8265 = session.isupport().casemapping().as_str() == "rfc7613";
    assert_eq!(warnings, usize::from(!supports_rfc8265));

    session.handle_line(now, b":ergo.test 005 papo NETWORK=Ergo :are supported");
    assert!(
        !std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::Utf8CasemappingUnsupported))
    );
}

#[test]
fn warns_once_when_isupport_storage_reaches_its_bound() {
    Transcript::new(Config::default())
        .connect_default()
        .update(|session| {
            let now = std::time::Instant::now();
            for index in 0..5_000 {
                session.handle_line(
                    now,
                    format!(":ergo.test 005 papo VENDOR{index}=present :are supported").as_bytes(),
                );
            }
            session.handle_line(
                now,
                b":ergo.test 005 papo CASEMAPPING=ascii PREFIX=(qaohv)~&@%+ CHANTYPES=# :are supported",
            );
        })
        .event(|event| {
            matches!(
                event,
                Event::StateLimitReached {
                    limit: StateLimit::IsupportTokens,
                    ..
                }
            )
        })
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .inspect(|session| {
            assert_eq!(session.isupport().tokens().len(), 1_024);
            assert_eq!(session.isupport().chantypes(), "#");
            assert_eq!(session.isupport().prefix().len(), 5);
        })
        .no_event();
}

fn join_release_with_alice(session: &mut Session, isupport: &str) {
    let now = std::time::Instant::now();
    let welcome =
        format!(":ergo.test 005 papo {isupport} CHANTYPES=# :are supported by this server");
    for line in [
        ":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo",
        welcome.as_str(),
        ":papo!~u@papo.irc JOIN #release",
        ":ergo.test 353 papo = #release :@papo alice[m]",
        ":ergo.test 366 papo #release :End of /NAMES list",
        ":alice{m}!~u@3qkv8a5h7d2xw.irc JOIN #release",
    ] {
        session.handle_line(now, line.as_bytes());
    }
}

#[test]
fn unknown_casemapping_keeps_bracket_nicks_apart() {
    let mut session = Session::new(Config::default().nick("papo"));
    join_release_with_alice(&mut session, "CASEMAPPING=rfc8265");

    let state = session.state();
    assert_eq!(state.casemapping(), papo_proto::Casemapping::Ascii);
    let channel = state.channel("#release").unwrap();
    assert_eq!(channel.members().len(), 3);
    assert_eq!(
        channel.member("alice[m]").unwrap().nick().as_str(),
        "alice[m]"
    );
    assert_eq!(
        channel.member("ALICE{M}").unwrap().nick().as_str(),
        "alice{m}"
    );
    assert_eq!(state.user("alice[m]").unwrap().host(), None);
    assert_eq!(
        state.user("alice{m}").unwrap().host(),
        Some("3qkv8a5h7d2xw.irc")
    );
    assert!(!session.__name_rules_known());
}

#[test]
fn unknown_casemapping_overrides_a_seeded_one() {
    let seeded = Config::default()
        .nick("papo")
        .casemapping(papo_proto::Casemapping::Rfc1459);
    let mut session = Session::new(seeded);
    join_release_with_alice(&mut session, "CASEMAPPING=rfc8265");

    assert_eq!(
        session.state().casemapping(),
        papo_proto::Casemapping::Ascii
    );
    assert_eq!(
        session.state().channel("#release").unwrap().members().len(),
        3
    );
    assert!(!session.__name_rules_known());
}

#[test]
fn known_casemapping_still_folds_brackets() {
    let mut session = Session::new(Config::default().nick("papo"));
    join_release_with_alice(&mut session, "CASEMAPPING=rfc1459");

    assert_eq!(
        session.state().channel("#release").unwrap().members().len(),
        2
    );
    assert!(session.__name_rules_known());
}

#[test]
fn casemapping_advertisements_set_rules_and_whether_they_are_known() {
    use papo_proto::Casemapping::{Ascii, Rfc1459};
    for (seed, token, rules, known) in [
        (Some(Ascii), "NETWORK=ErgoTest", Ascii, true),
        (None, "NETWORK=ErgoTest", Rfc1459, false),
        (None, "CASEMAPPING", Ascii, false),
        (Some(Rfc1459), "CASEMAPPING", Ascii, false),
        (Some(Ascii), "CASEMAPPING=rfc1459", Rfc1459, true),
    ] {
        let config = Config::default().nick("papo");
        let mut session = Session::new(match seed {
            Some(seed) => config.casemapping(seed),
            None => config,
        });
        join_release_with_alice(&mut session, token);
        assert_eq!(session.state().casemapping(), rules, "{seed:?} {token}");
        assert_eq!(session.__name_rules_known(), known, "{seed:?} {token}");
    }
}

#[test]
fn a_later_unknown_casemapping_makes_known_rules_unknown() {
    let now = std::time::Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    join_release_with_alice(&mut session, "CASEMAPPING=rfc1459");
    while session.poll_event().is_some() {}

    session.handle_line(
        now,
        b":ergo.test 005 papo CASEMAPPING=rfc8265 :are supported by this server",
    );

    assert_eq!(
        session.poll_event(),
        Some(Event::__casemapping_changed(
            papo_proto::Casemapping::Rfc1459,
            papo_proto::Casemapping::Ascii,
        ))
    );
    assert!(!session.__name_rules_known());
}

#[test]
fn seeded_rules_decide_who_we_are_before_any_005() {
    let now = std::time::Instant::now();
    let seeded = Config::default()
        .nick("papo[bot]")
        .casemapping(papo_proto::Casemapping::Ascii);
    let mut session = Session::new(seeded);
    session.handle_line(
        now,
        b":papo{bot}!~u@3qkv8a5h7d2xw.irc PRIVMSG #release :ship it",
    );
    session.handle_line(now, b":PAPO[bot]!~u@papo.irc PRIVMSG #release :shipping");

    let parties: Vec<_> = std::iter::from_fn(|| session.poll_event())
        .filter_map(|event| match event {
            Event::Line(line) => Some(line.actor_party()),
            _ => None,
        })
        .collect();
    assert_eq!(parties, [papo_core::Party::Other, papo_core::Party::Me]);
}
