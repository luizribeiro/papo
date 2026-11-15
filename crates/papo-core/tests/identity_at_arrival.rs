//! Transcript coverage for the identity an event records for the person it names.
//!
//! Each event is polled only after its subject left and another user took the nickname, so a
//! token read from the state at polling time would name the newcomer.

use std::time::Instant;

use papo_core::{Config, Event, ReplyToken, Session, TypingReason};

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
    session.monitor_add(["carol"]);
    for line in [
        ":papo!~papo@localhost JOIN #review",
        ":alice!~alice@cloak.example JOIN #review",
        ":mallory!~mallory@users.example JOIN #review",
        ":dave!~dave@users.example JOIN #review",
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
        Case {
            name: "CHGHOST",
            line: ":alice!~alice@cloak.example CHGHOST ~alice staff.example",
            subject: "alice",
            token: |event| match event {
                Event::HostChanged { token, .. } => Some(*token),
                _ => None,
            },
        },
        Case {
            name: "SETNAME",
            line: ":alice!~alice@cloak.example SETNAME :Alice Liddell",
            subject: "alice",
            token: |event| match event {
                Event::RealnameChanged { token, .. } => Some(*token),
                _ => None,
            },
        },
    ] {
        assert_identity_at_arrival(&case);
    }
}

#[test]
fn joins_and_renames_record_the_subject_at_arrival() {
    for case in [
        Case {
            name: "JOIN",
            line: ":carol!~carol@cloak.example JOIN #review",
            subject: "carol",
            token: |event| match event {
                Event::Joined { nick, token, .. } if nick == "carol" => Some(*token),
                _ => None,
            },
        },
        Case {
            name: "extended-join",
            line: ":carol!~carol@cloak.example JOIN #review carol :Carol Danvers",
            subject: "carol",
            token: |event| match event {
                Event::Joined { nick, token, .. } if nick == "carol" => Some(*token),
                _ => None,
            },
        },
        Case {
            name: "NICK",
            line: ":alice!~alice@cloak.example NICK alicia",
            subject: "alicia",
            token: |event| match event {
                Event::NickChanged { token, .. } => Some(*token),
                _ => None,
            },
        },
    ] {
        assert_identity_at_arrival(&case);
    }
}

#[test]
fn actors_and_subjects_record_the_person_at_arrival() {
    for case in [
        Case {
            name: "KICK actor",
            line: ":alice!~alice@cloak.example KICK #review dave :Flooding",
            subject: "alice",
            token: |event| match event {
                Event::Kicked { actor_token, .. } => Some(*actor_token),
                _ => None,
            },
        },
        Case {
            name: "MODE actor",
            line: ":alice!~alice@cloak.example MODE #review +v dave",
            subject: "alice",
            token: |event| match event {
                Event::ModeChanged { actor_token, .. } => Some(*actor_token),
                _ => None,
            },
        },
        Case {
            name: "user MODE subject",
            line: ":alice!~alice@cloak.example MODE alice +i",
            subject: "alice",
            token: |event| match event {
                Event::ModeChanged { subject_token, .. } => Some(*subject_token),
                _ => None,
            },
        },
        Case {
            name: "INVITE actor",
            line: ":alice!~alice@cloak.example INVITE papo #ops",
            subject: "alice",
            token: |event| match event {
                Event::Invited { actor_token, .. } => Some(*actor_token),
                _ => None,
            },
        },
        Case {
            name: "INVITE subject",
            line: ":dave!~dave@users.example INVITE alice #review",
            subject: "alice",
            token: |event| match event {
                Event::Invited { token, .. } => Some(*token),
                _ => None,
            },
        },
    ] {
        assert_identity_at_arrival(&case);
    }
}

#[test]
fn metadata_and_monitor_record_the_subject_at_arrival() {
    for case in [
        Case {
            name: "METADATA",
            line: ":ergo.test 761 papo alice display-name * :Alice Liddell",
            subject: "alice",
            token: |event| match event {
                Event::MetadataChanged { subject_token, .. } => Some(*subject_token),
                _ => None,
            },
        },
        Case {
            name: "MONITOR online",
            line: ":ergo.test 730 papo :carol!~carol@cloak.example",
            subject: "carol",
            token: |event| match event {
                Event::MonitorOnline { token, .. } => Some(*token),
                _ => None,
            },
        },
    ] {
        assert_identity_at_arrival(&case);
    }
}

#[test]
fn tags_redactions_and_typing_record_the_actor_at_arrival() {
    for case in [
        Case {
            name: "TAGMSG",
            line: "@+draft/react=👍;+draft/reply=RfcBTzyxY120001 :alice!~alice@cloak.example TAGMSG #review",
            subject: "alice",
            token: |event| match event {
                Event::TagMessage(message) => Some(message.actor_token),
                _ => None,
            },
        },
        Case {
            name: "REDACT",
            line: ":alice!~alice@cloak.example REDACT #review RfcBTzyxY120001 :off-topic",
            subject: "alice",
            token: |event| match event {
                Event::Redacted(redacted) => Some(redacted.actor_token),
                _ => None,
            },
        },
        Case {
            name: "typing",
            line: "@+typing=active :alice!~alice@cloak.example TAGMSG #review",
            subject: "alice",
            token: |event| match event {
                Event::TypingChanged(change) if change.reason == TypingReason::Notified => {
                    Some(change.__actor_token())
                }
                _ => None,
            },
        },
        Case {
            name: "typing stopped by the departure",
            line: "@+typing=active :alice!~alice@cloak.example TAGMSG #review",
            subject: "alice",
            token: |event| match event {
                Event::TypingChanged(change) if change.reason == TypingReason::Departed => {
                    Some(change.__actor_token())
                }
                _ => None,
            },
        },
    ] {
        assert_identity_at_arrival(&case);
    }
}
