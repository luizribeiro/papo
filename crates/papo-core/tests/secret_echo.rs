//! Transcript coverage for server echoes that repeat a secret we sent.

mod support;

use std::time::{Duration, Instant};

use papo_core::{Config, Event, RateLimit, Secret, Session};
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
