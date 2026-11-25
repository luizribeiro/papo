//! Transcript coverage for server echoes that repeat a secret we sent.

mod support;

use std::time::{Duration, Instant};

use papo_core::{Config, Event, RateLimit, RequestError, Secret, Session};
use papo_proto::Line;

const PASSWORD: &str = "services-password-5129";

fn session(now: Instant, server: &str, caps: &str) -> Session {
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .allow_insecure_auth(true)
            .request_timeout(Duration::from_secs(30))
            .rate_limit(RateLimit::unlimited()),
    );
    register(&mut session, now, server, caps);
    session
}

fn register(session: &mut Session, now: Instant, server: &str, caps: &str) {
    session.handle_connected(now);
    for line in [
        format!(":{server} CAP * LS :{caps}"),
        format!(":{server} CAP * ACK :{caps}"),
        format!(":{server} 001 papo :Welcome to the Test IRC Network papo"),
        format!(":{server} 422 papo :MOTD File is missing"),
    ] {
        session.handle_line(now, line.as_bytes());
    }
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
}

const ERGO_CAPS: &str = "echo-message labeled-response message-tags";

fn ergo(now: Instant) -> Session {
    session(now, "ergo.test", ERGO_CAPS)
}

fn identify() -> Line {
    Line::new("PRIVMSG")
        .param("NickServ")
        .param(Secret::new(format!("IDENTIFY alice {PASSWORD}")))
}

fn ergo_echo() -> String {
    format!(
        "@msgid=RfcBTzyxY120006;time=2026-11-17T10:02:11.482Z :papo!~papo@client.example PRIVMSG NickServ :IDENTIFY alice {PASSWORD}"
    )
}

fn echo_line(session: &mut Session) -> Line {
    std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::Line(received) => Some((*received).clone()),
            _ => None,
        })
        .expect("the echo is reported")
}

fn assert_redacted_echo(echo: &Line) {
    assert!(
        echo.get(1).is_some_and(|text| text.ends_with(PASSWORD)),
        "the application can still read its own echo"
    );
    assert!(
        format!("{echo:?}").ends_with("PRIVMSG NickServ :<redacted>"),
        "{echo:?}"
    );
}

#[test]
fn the_unlabelled_echo_of_a_secret_send_is_redacted() {
    let now = Instant::now();
    let mut session = ergo(now);
    session.send(now, identify()).unwrap();
    assert!(session.poll_transmit().is_some());
    session.handle_line(now, ergo_echo().as_bytes());
    support::queue_debug_marker(&mut session);
    support::assert_session_debug_redacts(&session, PASSWORD);

    assert_redacted_echo(&echo_line(&mut session));
    let rest = format!(
        "{:?}",
        std::iter::from_fn(|| session.poll_event()).collect::<Vec<_>>()
    );
    assert!(!rest.contains(PASSWORD), "{rest}");
}

#[test]
fn a_late_echo_of_a_timed_out_secret_request_is_redacted() {
    let now = Instant::now();
    let mut session = ergo(now);
    session.request(now, identify()).unwrap();
    while session.poll_transmit().is_some() {}
    let late = now + Duration::from_secs(31);
    session.handle_timeout(late);
    assert!(
        std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::RequestTimedOut { .. }))
    );

    session.handle_line(late, format!("@label=0;{}", &ergo_echo()[1..]).as_bytes());
    support::queue_debug_marker(&mut session);
    support::assert_session_debug_redacts(&session, PASSWORD);
    assert_redacted_echo(&echo_line(&mut session));
}

#[test]
fn another_source_repeating_a_secret_in_our_labelled_batch_stays_redacted() {
    let now = Instant::now();
    let mut session = session(now, "ergo.test", &format!("batch {ERGO_CAPS}"));
    session.request_silent(now, identify()).unwrap();
    while session.poll_transmit().is_some() {}
    for line in [
        "@label=0 :ergo.test BATCH +4 labeled-response".to_owned(),
        format!("@batch=4;{}", &ergo_echo()[1..]),
        format!(
            "@time=2026-11-17T10:02:11.483Z;batch=4 :NickServ!NickServ@localhost NOTICE papo :IDENTIFY alice {PASSWORD}"
        ),
        ":ergo.test BATCH -4".to_owned(),
    ] {
        session.handle_line(now, line.as_bytes());
    }
    support::queue_debug_marker(&mut session);
    support::assert_session_debug_redacts(&session, PASSWORD);

    let repeat = echo_line(&mut session);
    assert_eq!(repeat.as_ref().command(), papo_proto::Command::Notice);
    assert!(
        format!("{repeat:?}").ends_with("NOTICE papo :<redacted>"),
        "{repeat:?}"
    );
}

#[test]
fn an_ordinary_echo_and_other_text_stay_visible() {
    let now = Instant::now();
    let mut session = ergo(now);
    session.send(now, identify()).unwrap();
    session
        .send(now, Line::privmsg("NickServ", "INFO alice"))
        .unwrap();
    while session.poll_transmit().is_some() {}
    for line in [
        "@msgid=RfcBTzyxY120007;time=2026-11-17T10:02:12.482Z :papo!~papo@client.example PRIVMSG NickServ :INFO alice",
        ":NickServ!NickServ@services.test NOTICE papo :Information on alice (account alice):",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    let debug = format!(
        "{:?}",
        std::iter::from_fn(|| session.poll_event()).collect::<Vec<_>>()
    );
    assert!(debug.contains("PRIVMSG NickServ :INFO alice"), "{debug}");
    assert!(debug.contains("Information on alice"), "{debug}");
    assert!(!debug.contains("<redacted>"), "{debug}");
}

fn identify_as(account: usize) -> Line {
    Line::new("PRIVMSG")
        .param("NickServ")
        .param(Secret::new(format!("IDENTIFY alice{account} {PASSWORD}")))
}

fn echo_of(session: &mut Session, now: Instant, account: usize) -> String {
    session.handle_line(
        now,
        format!(
            "@msgid=RfcBTzyxY12{account:04};time=2026-11-17T10:02:11.482Z :papo!~papo@client.example PRIVMSG NickServ :IDENTIFY alice{account} {PASSWORD}"
        )
        .as_bytes(),
    );
    format!(
        "{:?}",
        std::iter::from_fn(|| session.poll_event()).collect::<Vec<_>>()
    )
}

#[test]
fn secret_sends_awaiting_their_echo_are_bounded_and_fail_closed() {
    let now = Instant::now();
    let mut session = ergo(now);
    let accepted = (0..10_000)
        .take_while(|&account| session.send(now, identify_as(account)).is_ok())
        .count();
    assert_eq!(accepted, 32);
    while session.poll_transmit().is_some() {}
    let tracked = format!("{session:?}").len();
    for account in 32..10_032 {
        assert_eq!(
            session.send(now, identify_as(account)),
            Err(RequestError::PendingLimit)
        );
    }
    assert!(session.poll_transmit().is_none());
    assert_eq!(format!("{session:?}").len(), tracked);
    session
        .send(now, Line::privmsg("NickServ", "INFO alice"))
        .unwrap();

    assert!(echo_of(&mut session, now, 1).contains(":<redacted>"));
    session.send(now, identify_as(32)).unwrap();
    assert_eq!(
        session.send(now, identify_as(33)),
        Err(RequestError::PendingLimit)
    );
    assert!(echo_of(&mut session, now, 0).contains(":<redacted>"));
    assert!(echo_of(&mut session, now, 1).contains(PASSWORD));
}

#[test]
fn a_line_counts_each_of_its_secrets() {
    let now = Instant::now();
    let mut session = ergo(now);
    for account in 0..30 {
        session.send(now, identify_as(account)).unwrap();
    }
    let two_secrets = |account: usize| {
        Line::new("NOTICE")
            .param("NickServ")
            .param(Secret::new(format!("{PASSWORD}-{account}")))
            .param(Secret::new(format!("staff key {account}")))
    };
    session.send(now, two_secrets(30)).unwrap();
    assert_eq!(
        session.send(now, identify_as(32)),
        Err(RequestError::PendingLimit)
    );
    assert!(echo_of(&mut session, now, 0).contains(":<redacted>"));
    assert_eq!(
        session.send(now, two_secrets(33)),
        Err(RequestError::PendingLimit)
    );
    session.send(now, identify_as(33)).unwrap();
}

#[test]
fn refused_secret_sends_make_room_once_their_echo_is_overdue() {
    let now = Instant::now();
    let mut session = ergo(now);
    for account in 0..32 {
        session.send(now, identify_as(account)).unwrap();
        session.handle_line(
            now,
            b":ergo.test 404 papo NickServ :Cannot send to user, you are not registered",
        );
    }
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
    let almost = now + Duration::from_secs(29);
    assert_eq!(
        session.send(almost, identify_as(32)),
        Err(RequestError::PendingLimit)
    );

    let overdue = now + Duration::from_secs(30);
    session.send(overdue, identify_as(32)).unwrap();
    assert!(echo_of(&mut session, overdue, 0).contains(PASSWORD));
    assert!(echo_of(&mut session, overdue, 1).contains(":<redacted>"));
}

#[test]
fn an_empty_secret_is_not_tracked() {
    let now = Instant::now();
    let mut session = ergo(now);
    for _ in 0..10_000 {
        session
            .send(
                now,
                Line::new("PRIVMSG")
                    .param("NickServ")
                    .param(Secret::new("")),
            )
            .unwrap();
    }
    session.handle_line(now, b":ergo.test 461 papo PRIVMSG :Not enough parameters");
    assert!(
        std::iter::from_fn(|| session.poll_event())
            .all(|event| !format!("{event:?}").contains("<redacted>"))
    );
}

#[test]
fn a_new_connection_lets_unanswered_secrets_make_room() {
    let now = Instant::now();
    let mut session = ergo(now);
    assert!((0..10_000).any(|_| session.send(now, identify_as(0)).is_err()));
    session.handle_disconnected(now);
    register(&mut session, now, "ergo.test", ERGO_CAPS);
    session.send(now, identify_as(1)).unwrap();
    assert!(echo_of(&mut session, now, 0).contains(":<redacted>"));
}

#[test]
fn secret_sends_the_server_never_echoes_are_not_tracked() {
    let now = Instant::now();
    let mut session = session(now, "solanum.test", "server-time");
    for account in 0..10_000 {
        session.send(now, identify_as(account)).unwrap();
    }
}

const REASON: &str = "pasted the staff key services-password-5129";
const REDACTED_MSGID: &str = "b56bhkz4gmj7ig2jeg9hs7d2cs";

fn ergo_redaction(now: Instant) -> Session {
    session(
        now,
        "ergo.test",
        "draft/message-redaction echo-message labeled-response message-tags server-time",
    )
}

fn ergo_redaction_without_echo_message(now: Instant) -> Session {
    session(
        now,
        "ergo.test",
        "draft/message-redaction labeled-response message-tags server-time",
    )
}

fn secret_redaction() -> Line {
    Line::new("REDACT")
        .param("#papo")
        .param(REDACTED_MSGID)
        .param(Secret::new(REASON))
}

fn ergo_redaction_echo() -> String {
    format!(
        "@msgid=qsgqkhqcmnxm6x2dcricaf69f6;time=2026-11-16T07:39:12.791Z :papo!~u@fa4ww98eysn9a.irc REDACT #papo {REDACTED_MSGID} :{REASON}"
    )
}

#[test]
fn the_echo_of_a_secret_redaction_reason_is_redacted() {
    let now = Instant::now();
    for connect in [ergo_redaction, ergo_redaction_without_echo_message] {
        assert_secret_redaction_echo_is_redacted(now, connect(now));
    }
}

fn assert_secret_redaction_echo_is_redacted(now: Instant, mut session: Session) {
    let id = session.request(now, secret_redaction()).unwrap();
    while session.poll_transmit().is_some() {}
    session.handle_line(now, ergo_redaction_echo().as_bytes());
    support::queue_debug_marker(&mut session);
    support::assert_session_debug_redacts(&session, PASSWORD);

    session.handle_line(
        now,
        b"@time=2026-11-16T07:39:12.791Z;label=0 :ergo.test ACK",
    );
    let events = std::iter::from_fn(|| session.poll_event()).collect::<Vec<_>>();
    let redacted = events
        .iter()
        .find_map(|event| match event {
            Event::Redacted(redacted) => Some(redacted),
            _ => None,
        })
        .expect("the echo reports the redaction");
    assert_eq!(redacted.msgid, REDACTED_MSGID);
    assert_eq!(redacted.reason.as_deref(), Some("<redacted>"));
    let response = events
        .iter()
        .find_map(|event| match event {
            Event::Response(response) if response.id == id => Some(response),
            _ => None,
        })
        .expect("the ACK completes the request");
    assert!(
        format!("{:?}", response.lines)
            .contains(&format!("REDACT #papo {REDACTED_MSGID} :<redacted>")),
        "{response:?}"
    );
    let debug = format!("{events:?}");
    assert!(!debug.contains(PASSWORD), "{debug}");
}

#[test]
fn a_refusal_repeating_a_secret_redaction_reason_is_redacted() {
    let now = Instant::now();
    let mut session = ergo_redaction(now);
    session.send(now, secret_redaction()).unwrap();
    while session.poll_transmit().is_some() {}
    session.handle_line(
        now,
        format!("@time=2026-11-16T07:39:19.107Z :ergo.test FAIL REDACT UNKNOWN_MSGID #papo {REDACTED_MSGID} :{REASON}")
            .as_bytes(),
    );
    support::queue_debug_marker(&mut session);
    support::assert_session_debug_redacts(&session, PASSWORD);
    let Some(Event::StandardReply(reply)) = session.poll_event() else {
        panic!("the refusal is reported");
    };
    assert_eq!(reply.code, "UNKNOWN_MSGID");
    assert_eq!(reply.context, ["#papo", REDACTED_MSGID]);
    assert_eq!(reply.description, "<redacted>");
}

#[test]
fn an_ordinary_redaction_reason_stays_visible() {
    let now = Instant::now();
    let mut session = ergo_redaction(now);
    session.send(now, secret_redaction()).unwrap();
    session.handle_line(
        now,
        format!("@msgid=4f6fb68i4ak2gs3dzkqdf2rwa6;time=2026-11-16T07:39:16.699Z :bob!~u@fa4ww98eysn9a.irc REDACT #papo {REDACTED_MSGID} :mod action")
            .as_bytes(),
    );
    session.handle_line(
        now,
        format!("@time=2026-11-16T07:39:19.107Z :ergo.test FAIL REDACT UNKNOWN_MSGID #papo {REDACTED_MSGID} :This message does not exist or is too old")
            .as_bytes(),
    );
    let events = std::iter::from_fn(|| session.poll_event()).collect::<Vec<_>>();
    assert!(events.iter().any(|event| matches!(event, Event::Redacted(redacted) if redacted.reason.as_deref() == Some("mod action"))), "{events:?}");
    assert!(events.iter().any(|event| matches!(event, Event::StandardReply(reply) if reply.description == "This message does not exist or is too old")), "{events:?}");
}

#[test]
fn credentials_stay_redacted_until_the_connection_ends() {
    let now = Instant::now();
    let mut session = ergo(now);
    let join = |key: Secret| Line::new("JOIN").param("#staff").param(key);

    let key = "staff-key-000";
    for index in 0..256 {
        session
            .send(now, join(Secret::new(format!("staff-key-{index:03}"))))
            .unwrap();
        assert!(session.poll_transmit().is_some());
    }
    session.send(now, join(Secret::new(key))).unwrap();
    assert!(
        session.poll_transmit().is_some(),
        "an already tracked key is accepted"
    );

    let late = now + Duration::from_secs(31);
    session.handle_timeout(late);
    session.handle_line(
        late,
        format!(":ergo.test WARN JOIN BAD_CHANNEL_KEY #staff :Rejected {key}").as_bytes(),
    );
    assert!(std::iter::from_fn(|| session.poll_event()).any(
        |event| matches!(event, Event::StandardReply(reply) if reply.description == "Rejected <redacted>"),
    ));
    let malformed = format!(":ergo.test NOTICE papo :Rejected {key}\0");
    session.handle_line(late, malformed.as_bytes());
    assert!(
        matches!(session.poll_event(), Some(Event::Malformed { line, .. }) if line == malformed.as_bytes())
    );

    let unrelated = b":ergo.test NOTICE papo :Invalid data\0";
    session.handle_line(late, unrelated);
    assert!(
        matches!(session.poll_event(), Some(Event::Malformed { line, .. }) if line == unrelated)
    );

    let new = join(Secret::new("staff-key-256"));
    let token = session.send_tracked(late, new.clone()).unwrap();
    assert!(
        session.poll_transmit().is_none(),
        "an untracked key cannot go on the wire"
    );
    assert!(matches!(
        session.poll_event(),
        Some(Event::TransmitCredentialsRefused {
            token: refused,
            error: RequestError::StateLimit(papo_core::StateLimit::Credentials),
            ..
        }) if refused == token
    ));
    session.send(late, join(Secret::new(key))).unwrap();
    assert!(session.poll_transmit().is_some());
    register(&mut session, late, "ergo.test", ERGO_CAPS);
    session.send(late, new).unwrap();
    assert!(session.poll_transmit().is_some());
}

#[test]
fn channel_mode_replies_preserve_keys_only_in_typed_state() {
    let now = Instant::now();
    let mut session = ergo(now);
    let key = "staff-snapshot-key-4821";
    session
        .send(
            now,
            Line::new("JOIN").param("#staff").param(Secret::new(key)),
        )
        .unwrap();
    assert_eq!(
        &*session.poll_transmit().unwrap(),
        format!("JOIN #staff {key}\r\n").as_bytes()
    );
    session.handle_line(
        now,
        b":ergo.test 005 papo CHANMODES=beI,k,l,imnpst :are supported by this server",
    );
    session.handle_line(now, b":papo!~papo@localhost JOIN #staff");
    session.handle_line(
        now,
        format!(":operator!~o@services.example MODE #staff +k {key}").as_bytes(),
    );
    let mode = std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::ModeChanged { changes, .. } => Some(changes),
            _ => None,
        })
        .unwrap();
    assert!(
        matches!(&mode[0].arg, Some(papo_core::ModeArg::Key(secret)) if secret.expose() == key)
    );
    assert!(!format!("{mode:?}").contains(key));
    while session.poll_transmit().is_some() {}
    session
        .request(now, Line::new("MODE").param("#staff"))
        .unwrap();
    assert_eq!(
        &*session.poll_transmit().unwrap(),
        b"@label=0 MODE #staff\r\n"
    );
    session.handle_line(
        now,
        format!("@label=0 :ergo.test 324 papo #staff +lk 25 {key}").as_bytes(),
    );
    assert_eq!(
        session
            .state()
            .channel("#staff")
            .unwrap()
            .key()
            .map(Secret::expose),
        Some(key)
    );
    let response = std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::Response(response) => Some(response),
            _ => None,
        })
        .unwrap();
    assert_eq!(response.lines[0].get(3), Some("25"));
    assert_eq!(response.lines[0].get(4), Some("<redacted>"));
    assert!(!format!("{response:?} {session:?}").contains(key));
}

#[test]
fn credential_collisions_leave_ordinary_server_lines_intact() {
    let now = Instant::now();
    for key in ["papo", "rust", "a"] {
        let mut reference = ergo(now);
        let mut session = ergo(now);
        session
            .send(
                now,
                Line::new("JOIN").param("#rust").param(Secret::new(key)),
            )
            .unwrap();
        assert!(session.poll_transmit().is_some());
        for line in [
            ":ergo.test 001 papo :Welcome to the rust network, papo",
            ":papo!~papo@localhost JOIN #rust",
            ":ergo.test 353 papo = #rust :@ferris papo rustacean alice",
            ":ergo.test 366 papo #rust :End of /NAMES list.",
            ":ergo.test 332 papo #rust :Welcome to rust, read the book before asking",
            ":ergo.test 372 papo :- This network runs on rust and coffee",
            ":alice!~alice@users.example NOTICE #rust :rust meetup tonight at eight",
            ":alice!~alice@users.example PRIVMSG #rust :the rust release is out",
        ] {
            reference.handle_line(now, line.as_bytes());
            session.handle_line(now, line.as_bytes());
            assert!(
                session.state() == reference.state(),
                "key {key}, line {line}"
            );
            let events = |session: &mut Session| {
                format!(
                    "{:?}",
                    std::iter::from_fn(|| session.poll_event()).collect::<Vec<_>>()
                )
            };
            assert_eq!(
                events(&mut session),
                events(&mut reference),
                "key {key}, line {line}"
            );
        }
    }
}

fn keyed_staff_join(key: &str) -> Line {
    Line::new("JOIN").param("#staff").param(Secret::new(key))
}

#[test]
fn an_unsolicited_failure_keeps_its_addressee_and_unrelated_text() {
    let now = Instant::now();
    let mut session = ergo(now);
    for key in ["papo", "#staff", "a"] {
        session.send(now, keyed_staff_join(key)).unwrap();
        assert!(session.poll_transmit().is_some());
    }
    session.handle_line(
        now,
        b":ergo.test 696 papo #staff k #staff :Invalid mode parameter",
    );
    let line = echo_line(&mut session);
    assert_eq!(
        line.as_ref().params().collect::<Vec<_>>(),
        [
            "papo",
            "<redacted>",
            "k",
            "<redacted>",
            "Invalid mode parameter"
        ]
    );
    session.handle_line(
        now,
        b":ergo.test 696 papo #staff k papo :Invalid mode parameter",
    );
    let line = echo_line(&mut session);
    assert_eq!(line.get(0), Some("papo"));
    assert_eq!(line.get(3), Some("<redacted>"));
    session.handle_line(
        now,
        br#":ergo.test NOTE JOIN CHANNEL_STATE #staff a :Channel status "open""#,
    );
    let Some(Event::StandardReply(reply)) = session.poll_event() else {
        panic!("expected a standard reply")
    };
    assert_eq!(reply.context, ["<redacted>", "<redacted>"]);
    assert_eq!(reply.description, r#"Channel status "open""#);
    session.handle_line(
        now,
        b":ergo.test WARN JOIN BAD_CHANNEL_KEY #staff :Rejected 'a'",
    );
    let Some(Event::StandardReply(reply)) = session.poll_event() else {
        panic!("expected a standard reply");
    };
    assert_eq!(reply.description, "Rejected '<redacted>'");
}

#[test]
fn refusing_a_line_does_not_reserve_any_of_its_new_credentials() {
    let now = Instant::now();
    let mut session = ergo(now);
    for index in 0..255 {
        session
            .send(
                now,
                Line::new("JOIN")
                    .param("#staff")
                    .param(Secret::new(format!("staff-key-{index:03}"))),
            )
            .unwrap();
        assert!(session.poll_transmit().is_some());
    }
    let mode = |first, second| {
        Line::new("MODE")
            .with_params(["#staff", "+kk"])
            .param(Secret::new(first))
            .param(Secret::new(second))
    };
    session
        .send(now, mode("one-more-4821", "over-the-limit-4821"))
        .unwrap();
    assert!(session.poll_transmit().is_none());
    session
        .send(now, mode("last-key-4821", "last-key-4821"))
        .unwrap();
    assert!(session.poll_transmit().is_some());
}

#[test]
fn failure_replies_mask_whole_parameters_and_quoted_tokens() {
    let now = Instant::now();
    let key = "#staff-key-4821";
    for (head, description, index) in [
        ("475 papo #staff", "Cannot join channel with key", 2),
        ("696 papo #staff k", "Invalid mode k parameter", 3),
        ("464 papo", "Password incorrect", 1),
        ("904 papo", "SASL authentication failed", 1),
        ("FAIL JOIN BAD_CHANNEL_KEY #staff", "Invalid channel key", 3),
    ] {
        let mut session = ergo(now);
        session.request(now, keyed_staff_join(key)).unwrap();
        assert!(session.poll_transmit().is_some());
        session.handle_line(
            now,
            format!("@label=0 :ergo.test {head} {key} :{description} \"{key}\" supplied")
                .as_bytes(),
        );
        let events = std::iter::from_fn(|| session.poll_event()).collect::<Vec<_>>();
        let response = events
            .iter()
            .find_map(|event| match event {
                Event::Response(response) => Some(response),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{head}: {events:?}"));
        let line = &response.lines[0];
        assert_eq!(line.get(index), Some("<redacted>"));
        assert_eq!(
            line.as_ref().params().last().unwrap(),
            format!("{description} \"<redacted>\" supplied")
        );
        assert!(!format!("{events:?} {:?} {line}", response.error()).contains(key));
    }
    for (reason, expected) in [
        (key.to_owned(), "<redacted>"),
        (
            format!("Closing link: bad password '{key}'"),
            "Closing link: bad password '<redacted>'",
        ),
    ] {
        let mut session = ergo(now);
        session.send(now, keyed_staff_join(key)).unwrap();
        assert!(session.poll_transmit().is_some());
        session.handle_line(now, format!("ERROR :{reason}").as_bytes());
        assert!(
            matches!(session.poll_event(), Some(Event::Close { reason, .. }) if reason == expected)
        );
    }
}

#[test]
fn refused_requests_keep_only_the_matched_names() {
    use papo_core::ResponseMatcher;

    let nick = || "ferris".to_owned();
    let channel = || "#staff".to_owned();
    let cases = [
        (
            433,
            "NICK ferris",
            ResponseMatcher::Nick(nick()),
            &["ferris"][..],
        ),
        (
            401,
            "WHOIS ferris",
            ResponseMatcher::Whois(nick()),
            &["ferris"],
        ),
        (
            443,
            "INVITE ferris #staff",
            ResponseMatcher::__invite(channel(), nick()),
            &["ferris", "#staff"],
        ),
        (
            441,
            "KICK #staff ferris",
            ResponseMatcher::__kick(channel(), nick()),
            &["ferris", "#staff"],
        ),
        (
            480,
            "JOIN #staff",
            ResponseMatcher::Join(channel()),
            &["#staff"],
        ),
    ];
    for (numeric, sent, matcher, names) in cases {
        let now = Instant::now();
        let mut session = session(now, "ergo.test", "multi-prefix");
        for key in ["papo", "ferris", "#staff"] {
            session.send(now, keyed_staff_join(key)).unwrap();
            assert!(session.poll_transmit().is_some());
        }
        let id = session
            .request_with_matcher(now, sent.parse().unwrap(), matcher)
            .unwrap();
        assert_eq!(
            &*session.poll_transmit().unwrap(),
            format!("{sent}\r\n").as_bytes()
        );
        session.handle_line(
            now,
            format!(
                ":ergo.test {numeric} papo {} :Request for ferris on #staff rejected by papo",
                names.join(" ")
            )
            .as_bytes(),
        );
        let response = std::iter::from_fn(|| session.poll_event())
            .find_map(|event| match event {
                Event::Response(response) if response.id == id => Some(response),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no reply to {sent}"));
        let expected = ["papo"]
            .into_iter()
            .chain(names.iter().copied())
            .chain(["Request for <redacted> on <redacted> rejected by <redacted>"])
            .collect::<Vec<_>>();
        assert_eq!(
            response.lines[0].as_ref().params().collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn a_labelled_batch_keeps_the_requested_name_and_hides_the_echoed_key() {
    let now = Instant::now();
    let mut session = session(now, "ergo.test", "batch labeled-response message-tags");
    for key in ["ferris", "staff-key-4821"] {
        session.send(now, keyed_staff_join(key)).unwrap();
        assert!(session.poll_transmit().is_some());
    }
    let id = session
        .request_with_matcher(
            now,
            Line::new("NICK").param("ferris"),
            papo_core::ResponseMatcher::Nick("ferris".into()),
        )
        .unwrap();
    assert_eq!(
        &*session.poll_transmit().unwrap(),
        b"@label=0 NICK ferris\r\n"
    );
    session.handle_line(now, b"@label=0 :ergo.test BATCH +refusal labeled-response");
    session.handle_line(
        now,
        b"@batch=refusal :ergo.test 433 papo ferris staff-key-4821 :Nickname is already in use",
    );
    let debug = format!("{session:?}");
    assert!(!debug.contains("staff-key-4821") && debug.contains("433 papo <redacted> <redacted>"));
    assert!(session.poll_event().is_none());
    session.handle_line(now, b":ergo.test BATCH -refusal");
    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("expected the completed batch");
    };
    assert_eq!(response.id, id);
    assert_eq!(response.lines[0].get(1), Some("ferris"));
    assert_eq!(response.lines[0].get(2), Some("<redacted>"));
    assert!(!format!("{response:?} {:?}", response.error()).contains("staff-key-4821"));
}

#[test]
fn refusing_a_credential_discards_only_its_mode_request_and_fence() {
    let now = Instant::now();
    let mut session = session(now, "ergo.test", "multi-prefix");
    for index in 0..256 {
        let key = format!("staff-key-{index:03}");
        session.send(now, keyed_staff_join(&key)).unwrap();
        assert!(session.poll_transmit().is_some());
    }
    let id = session
        .request_with_matcher(
            now,
            Line::new("MODE")
                .with_params(["#staff", "+k"])
                .param(Secret::new("new-staff-key")),
            papo_core::ResponseMatcher::Mode("#staff".into()),
        )
        .unwrap();
    session
        .send(now, Line::new("PING").param("connection-probe"))
        .unwrap();

    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"PING connection-probe\r\n"
    );
    assert!(session.poll_transmit().is_none());
    assert!(matches!(session.poll_event(),
        Some(Event::RequestFailed(refused, RequestError::StateLimit(papo_core::StateLimit::Credentials)))
        if refused == id
    ));
}

#[test]
fn malformed_ordinary_lines_keep_their_bytes() {
    let now = Instant::now();
    let mut session = ergo(now);
    session.send(now, keyed_staff_join("a")).unwrap();
    assert!(session.poll_transmit().is_some());
    session.handle_line(
        now,
        b":ergo.test 005 papo UTF8ONLY :are supported by this server",
    );
    while session.poll_event().is_some() {}
    let ordinary = b":alice!~alice@users.example PRIVMSG #rust :caf\xe9 at eight";
    session.handle_line(now, ordinary);
    assert!(
        matches!(session.poll_event(), Some(Event::Malformed { line, .. }) if line == ordinary)
    );
    session.handle_line(now, b":ergo.test 696 papo #rust k a :Cl\xe9 invalide");
    assert!(
        matches!(session.poll_event(), Some(Event::Malformed { line, .. })
        if line == b":ergo.test 696 papo #rust k <redacted> :Cl\xe9 invalide")
    );
}

#[test]
fn punctuation_bounds_failure_reply_credentials() {
    let now = Instant::now();
    let mut session = ergo(now);
    session
        .send(now, keyed_staff_join("staff-key-4821"))
        .unwrap();
    assert!(session.poll_transmit().is_some());
    for (token, expected) in [
        ("(staff-key-4821)", "(<redacted>)"),
        ("staff-key-4821.", "<redacted>."),
        ("staff-key-4821,", "<redacted>,"),
        ("staff-key-4821:", "<redacted>:"),
        ("'staff-key-4821'.", "'<redacted>'."),
        ("key=<staff-key-4821>", "key=<<redacted>>"),
        ("staff-key-4821x", "staff-key-4821x"),
    ] {
        let reply = format!(":ergo.test 696 papo #staff k {token} :Rejected {token}");
        session.handle_line(now, reply.as_bytes());
        let line = echo_line(&mut session);
        assert_eq!(line.get(3), Some(expected));
        assert_eq!(line.get(4).unwrap(), format!("Rejected {expected}"));
    }
}

#[test]
fn buffered_failure_bytes_stay_out_of_session_debug() {
    let now = Instant::now();
    let mut session = ergo(now);
    session
        .send(now, keyed_staff_join("staff-key-4821"))
        .unwrap();
    assert!(session.poll_transmit().is_some());
    let partial = b":ergo.test 475 papo #staff :Invalid key staff-key-4821";
    session.handle_bytes(now, partial);
    let debug = format!("{session:?}");
    assert!(debug.contains(&format!("buffered_bytes: {}", partial.len())));
    assert!(!debug.contains(&format!("{partial:?}")));
    session.handle_bytes(now, b"\r\n");
    assert_eq!(
        echo_line(&mut session).get(2),
        Some("Invalid key <redacted>")
    );
}
