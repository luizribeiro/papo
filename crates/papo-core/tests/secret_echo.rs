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
        |event| matches!(event, Event::StandardReply(reply) if reply.description == "<redacted>"),
    ));
    session.handle_line(
        late,
        format!(":ergo.test NOTICE papo :Rejected {key}\0").as_bytes(),
    );
    assert!(
        matches!(session.poll_event(), Some(Event::Malformed { line, .. }) if line == b"<redacted>")
    );

    let unrelated = b":ergo.test NOTICE papo :Invalid data\0";
    session.handle_line(late, unrelated);
    assert!(
        matches!(session.poll_event(), Some(Event::Malformed { line, .. }) if line == unrelated)
    );

    let new = join(Secret::new("staff-key-256"));
    session.send(late, new.clone()).unwrap();
    assert!(
        session.poll_transmit().is_none(),
        "an untracked key cannot go on the wire"
    );
    assert!(matches!(session.poll_event(), Some(Event::Close { .. })));
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
