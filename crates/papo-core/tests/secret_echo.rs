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
