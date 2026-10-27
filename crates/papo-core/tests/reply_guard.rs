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
    joined_session_with_origin(papo_proto::Origin::NONE)
}

fn joined_session_with_origin(origin: papo_proto::Origin) -> Session {
    let mut config = Config::default().nick("papo");
    config.origin = origin;
    let mut session = Session::new(config);
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

#[test]
fn a_reply_token_from_another_origin_is_always_stale() {
    let mut first = joined_session_with_origin(papo_proto::Origin::__new(41));
    let receipt = private_message(&mut first);
    let second = joined_session_with_origin(papo_proto::Origin::__new(42));

    assert!(second.state().__recipient_changed("alice", receipt));
}

fn private_message(session: &mut Session) -> ReplyToken {
    server(
        session,
        "@account=account :alice!~alice@cloak.example PRIVMSG papo :release approval",
    );
    loop {
        match session.poll_event() {
            Some(Event::Line(message)) => return message.__reply_token(),
            Some(_) => {}
            None => panic!("expected private message"),
        }
    }
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
