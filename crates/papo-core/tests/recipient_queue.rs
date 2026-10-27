//! Transcript coverage for queued direct recipient validation.

use std::time::Instant;

use papo_core::{Account, Config, Event, RateLimit, Session};
use papo_proto::Line;

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

fn discard_events(session: &mut Session) {
    while session.poll_event().is_some() {}
}

#[test]
fn queued_direct_message_follows_the_same_users_nick_change() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::new(1, std::time::Duration::from_secs(1)).unwrap()),
    );
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(&mut session, ":alice!~alice@cloak.example JOIN #review");
    discard_events(&mut session);
    let guard = session.__direct_acquire(
        "alice",
        Some("~alice"),
        Some("cloak.example"),
        Account::LoggedOut,
    );

    session.send(now, Line::new("PING").param("fence")).unwrap();
    session
        .send_guarded_tracked(now, Line::privmsg("alice", "release approved"), Some(guard))
        .unwrap();
    assert_eq!(session.poll_transmit().unwrap().as_ref(), b"PING fence\r\n");
    server(&mut session, ":alice!~alice@cloak.example NICK alicia");
    session
        .send(
            now + std::time::Duration::from_secs(1),
            Line::new("PING").param("advance"),
        )
        .unwrap();

    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"PRIVMSG alicia :release approved\r\n"
    );
}

#[test]
fn queued_direct_message_is_dropped_after_a_nickname_takeover() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::new(1, std::time::Duration::from_secs(1)).unwrap()),
    );
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(&mut session, ":alice!~alice@cloak.example JOIN #review");
    server(&mut session, ":mallory!~mallory@users.example JOIN #review");
    discard_events(&mut session);
    let guard = session.__direct_acquire(
        "alice",
        Some("~alice"),
        Some("cloak.example"),
        Account::LoggedOut,
    );

    session.send(now, Line::new("PING").param("fence")).unwrap();
    let token = session
        .send_guarded_tracked(now, Line::notice("alice", "classified"), Some(guard))
        .unwrap();
    _ = session.poll_transmit();
    server(
        &mut session,
        ":alice!~alice@cloak.example QUIT :Client Quit",
    );
    server(&mut session, ":mallory!~mallory@users.example NICK alice");
    discard_events(&mut session);
    session
        .send(
            now + std::time::Duration::from_secs(1),
            Line::new("PING").param("advance"),
        )
        .unwrap();

    assert_eq!(session.poll_transmit(), None);
    assert!(matches!(
        session.poll_event(),
        Some(Event::TransmitRecipientGone { token: gone, .. }) if gone == token
    ));
}
