//! Transcript coverage for private reply recipient validation.

use std::time::Instant;

use papo_core::{Config, Event, ReplyToken, Session};

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

fn discard_events(session: &mut Session) {
    while session.poll_event().is_some() {}
}

fn joined_session() -> Session {
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, ":papo!~papo@localhost JOIN #review");
    server(
        &mut session,
        ":alice!~alice@cloak.example JOIN #review account :Alice Example",
    );
    server(
        &mut session,
        ":mallory!~mallory@users.example JOIN #review * :Mallory Example",
    );
    discard_events(&mut session);
    session
}

fn private_message(session: &mut Session) -> ReplyToken {
    server(
        session,
        "@account=account :alice!~alice@cloak.example PRIVMSG papo :release approval",
    );
    let Some(Event::Line(message)) = session.poll_event() else {
        panic!("expected private message")
    };
    message.__reply_token()
}

#[test]
fn detects_a_private_nick_taken_after_its_sender_quits() {
    let mut session = joined_session();
    let receipt = private_message(&mut session);
    assert!(!session.state().__recipient_changed("alice", receipt));

    server(
        &mut session,
        ":alice!~alice@cloak.example QUIT :Client Quit",
    );
    server(&mut session, ":mallory!~mallory@users.example NICK alice");

    assert!(session.state().__recipient_changed("alice", receipt));
}

#[test]
fn detects_a_private_nick_taken_after_its_sender_renames() {
    let mut session = joined_session();
    let receipt = private_message(&mut session);

    server(&mut session, ":alice!~alice@cloak.example NICK alicia");
    server(&mut session, ":mallory!~mallory@users.example NICK alice");

    assert!(session.state().__recipient_changed("alice", receipt));
}

#[test]
fn permits_the_sender_to_rename_away_and_back() {
    let mut session = joined_session();
    let receipt = private_message(&mut session);

    server(&mut session, ":alice!~alice@cloak.example NICK alicia");
    server(&mut session, ":alicia!~alice@cloak.example NICK alice");

    assert!(!session.state().__recipient_changed("alice", receipt));
}

#[test]
fn permits_a_reply_when_the_recipient_is_unknown() {
    let mut session = Session::new(Config::default().nick("papo"));
    let receipt = private_message(&mut session);

    assert!(!session.state().__recipient_changed("alice", receipt));
}

#[test]
fn bounded_departure_history_fails_open_after_evidence_is_evicted() {
    let mut session = joined_session();
    let receipt = private_message(&mut session);
    server(
        &mut session,
        ":alice!~alice@cloak.example QUIT :Client Quit",
    );
    server(&mut session, ":mallory!~mallory@users.example NICK alice");
    for index in 0..254 {
        server(
            &mut session,
            &format!(":guest{index}!~guest@users.example QUIT :Connection closed"),
        );
    }
    assert!(session.state().__recipient_changed("alice", receipt));

    server(
        &mut session,
        ":overflow!~overflow@users.example QUIT :Connection closed",
    );
    assert!(!session.state().__recipient_changed("alice", receipt));
}

#[test]
fn a_connection_boundary_discards_old_departure_evidence() {
    let mut session = joined_session();
    let receipt = private_message(&mut session);
    session.handle_connected(Instant::now());
    server(
        &mut session,
        ":alice!~new-alice@users.example JOIN #review * :New Alice",
    );

    assert!(!session.state().__recipient_changed("alice", receipt));
}
