//! Transcript coverage for the identity an event records for the person it names.
//!
//! Each event is polled only after its subject left and another user took the nickname, so a
//! token read from the state at polling time would name the newcomer.

use std::time::Instant;

use papo_core::{Config, Event, ReplyToken, Session};

struct Case {
    name: &'static str,
    line: &'static str,
    subject: &'static str,
    token: fn(&Event) -> Option<ReplyToken>,
}

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

fn drain(session: &mut Session) {
    while session.poll_event().is_some() {}
}

fn joined_review() -> Session {
    let mut session = Session::new(Config::default().nick("papo"));
    for line in [
        ":papo!~papo@localhost JOIN #review",
        ":alice!~alice@cloak.example JOIN #review",
        ":mallory!~mallory@users.example JOIN #review",
    ] {
        server(&mut session, line);
    }
    drain(&mut session);
    session
}

fn assert_identity_at_arrival(case: &Case) {
    let mut session = joined_review();
    server(&mut session, case.line);
    let at_arrival = session.state().__reply_token_for(case.subject);
    assert!(at_arrival.__is_known(), "{}: subject is unknown", case.name);
    server(
        &mut session,
        &format!(":{}!~alice@cloak.example QUIT :Quit: bye", case.subject),
    );
    server(
        &mut session,
        &format!(":mallory!~mallory@users.example NICK {}", case.subject),
    );
    let newcomer = session.state().__reply_token_for(case.subject);
    assert_ne!(at_arrival, newcomer, "{}: no handoff happened", case.name);

    let recorded = std::iter::from_fn(|| session.poll_event())
        .find_map(|event| (case.token)(&event))
        .unwrap_or_else(|| panic!("{}: no event", case.name));
    assert_eq!(recorded, at_arrival, "{}", case.name);
}

#[test]
fn presence_changes_record_the_subject_at_arrival() {
    for case in [
        Case {
            name: "account-notify",
            line: ":alice!~alice@cloak.example ACCOUNT alice",
            subject: "alice",
            token: |event| match event {
                Event::AccountChanged { token, .. } => Some(*token),
                _ => None,
            },
        },
        Case {
            name: "account tag",
            line: "@account=alice :alice!~alice@cloak.example PRIVMSG #review :ready for review",
            subject: "alice",
            token: |event| match event {
                Event::AccountChanged { token, .. } => Some(*token),
                _ => None,
            },
        },
        Case {
            name: "away-notify",
            line: ":alice!~alice@cloak.example AWAY :Back in five",
            subject: "alice",
            token: |event| match event {
                Event::AwayChanged { token, .. } => Some(*token),
                _ => None,
            },
        },
        Case {
            name: "RPL_AWAY",
            line: ":ergo.test 301 papo alice :Back in five",
            subject: "alice",
            token: |event| match event {
                Event::AwayChanged { token, .. } => Some(*token),
                _ => None,
            },
        },
    ] {
        assert_identity_at_arrival(&case);
    }
}
