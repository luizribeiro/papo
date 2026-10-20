//! Transcript coverage for retained typing state.

mod support;

use std::time::Instant;

use papo_core::{Config, Conversation, Event, Session};
use papo_proto::Typing;

fn conversation(session: &Session, target: &str) -> Conversation {
    session.state().parse_target(target).unwrap().1
}

#[test]
fn notifications_update_state_in_start_order_without_refresh_churn() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    for line in [
        "@+typing=active :alice!~a@cloak TAGMSG #papo",
        "@+typing=paused :bob!~b@cloak TAGMSG #papo",
    ] {
        session.handle_line(now, line.as_bytes());
        _ = session.poll_event();
    }
    let generation = session.state_generation();
    session.handle_line(now, b"@+typing=active :ALICE!~a@cloak TAGMSG #papo");
    assert_eq!(session.state_generation(), generation);
    _ = session.poll_event();

    let conversation = conversation(&session, "#papo");
    let typers: Vec<_> = session.state().typing(&conversation).collect();
    assert_eq!(typers.len(), 2);
    assert_eq!(typers[0].nick, "alice");
    assert_eq!(typers[0].state, Typing::Active);
    assert_eq!(typers[1].nick, "bob");

    session.handle_line(now, b"@+typing=done :alice!~a@cloak TAGMSG #papo");
    _ = session.poll_event();
    let remaining: Vec<_> = session.state().typing(&conversation).collect();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].nick, "bob");
}

#[test]
fn own_and_history_typing_are_not_stored() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    session.handle_line(
        now,
        b"@+typing=active :papo!other@session.example TAGMSG #papo",
    );
    assert!(matches!(
        session.poll_event(),
        Some(Event::TypingChanged(_))
    ));

    session.handle_line(now, b"BATCH +history chathistory #papo");
    session.handle_line(
        now,
        b"@batch=history;+typing=active :alice!~a@cloak TAGMSG #papo",
    );
    session.handle_line(now, b"BATCH -history");
    assert!(matches!(session.poll_event(), Some(Event::History { .. })));

    let conversation = conversation(&session, "#papo");
    assert_eq!(session.state().typing(&conversation).count(), 0);
}
