//! Transcript and deadline tests for connection registration.

use std::time::{Duration, Instant};

use papo_core::{Config, Event, Session};

fn line(session: &mut Session) -> String {
    String::from_utf8(session.poll_transmit().expect("client line"))
        .unwrap()
        .trim_end()
        .to_owned()
}

#[test]
fn sends_registration_in_protocol_order() {
    let now = Instant::now();
    let config = Config::default()
        .nick("emily")
        .username("em")
        .realname("Emily Example")
        .password("correct horse battery staple");
    let mut session = Session::new(config);

    session.handle_connected(now);

    assert_eq!(line(&mut session), "CAP LS 302");
    assert_eq!(line(&mut session), "PASS :correct horse battery staple");
    assert_eq!(line(&mut session), "NICK emily");
    assert_eq!(line(&mut session), "USER em 0 * :Emily Example");
    assert_eq!(session.poll_transmit(), None);
}

#[test]
fn welcome_adopts_the_server_nick_and_cancels_the_timeout() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("oldnick"));
    session.handle_connected(now);

    session.handle_line(
        now + Duration::from_secs(1),
        b":irc.example 001 account :Welcome to the network",
    );

    assert_eq!(session.nick(), "account");
    assert_eq!(session.poll_timeout(), None);
}

#[test]
fn registration_timeout_is_the_first_deadline_and_closes_once() {
    let now = Instant::now();
    let timeout = Duration::from_secs(23);
    let mut session = Session::new(Config::default().registration_timeout(timeout));
    session.handle_connected(now);
    assert_eq!(session.poll_timeout(), Some(now + timeout));

    session.handle_timeout(now + timeout.checked_sub(Duration::from_nanos(1)).unwrap());
    assert_eq!(session.poll_event(), None);
    assert_eq!(session.poll_timeout(), Some(now + timeout));

    session.handle_timeout(now + timeout);
    assert!(matches!(
        session.poll_event(),
        Some(Event::Close { reason }) if reason.contains("before 001")
    ));
    assert_eq!(session.poll_timeout(), None);
    session.handle_timeout(now + timeout);
    assert_eq!(session.poll_event(), None);
}
