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
    session.handle_connected(now);
    for line in [
        "@+typing=active :alice!~a@cloak TAGMSG #papo",
        "@+typing=paused :bob!~b@cloak TAGMSG #papo",
    ] {
        session.handle_line(now, line.as_bytes());
        _ = session.poll_event();
    }
    let generation = session.state_generation();
    session.handle_line(
        now + std::time::Duration::from_secs(3),
        b"@+typing=active :ALICE!~a@cloak TAGMSG #papo",
    );
    assert_eq!(session.state_generation(), generation);
    assert_eq!(
        session.poll_timeout(),
        Some(now + std::time::Duration::from_secs(9))
    );
    _ = session.poll_event();

    let conversation = conversation(&session, "#papo");
    let typers: Vec<_> = session.state().typing(&conversation).collect();
    assert_eq!(typers.len(), 2);
    assert_eq!(typers[0].nick, "alice");
    assert_eq!(typers[0].state, Typing::Active);
    assert_eq!(typers[1].nick, "bob");

    session.handle_timeout(now + std::time::Duration::from_secs(8));
    assert_eq!(session.state_generation(), generation);
    session.handle_timeout(now + std::time::Duration::from_secs(9));
    assert!(matches!(
        session.poll_event(),
        Some(Event::TypingChanged(change))
            if change.state == Typing::Done && change.reason == papo_core::TypingReason::TimedOut
    ));
    assert_eq!(session.state_generation(), generation + 1);
    let remaining: Vec<_> = session.state().typing(&conversation).collect();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].nick, "bob");
}

#[test]
fn paused_typing_expires_after_thirty_seconds() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_connected(now);
    session.handle_line(now, b"@+typing=paused :alice!~a@cloak TAGMSG #papo");
    _ = session.poll_event();
    assert_eq!(
        session.poll_timeout(),
        Some(now + std::time::Duration::from_secs(30))
    );

    session.handle_timeout(now + std::time::Duration::from_secs(29));
    assert_eq!(session.state_generation(), 2);
    session.handle_timeout(now + std::time::Duration::from_secs(30));
    assert!(matches!(
        session.poll_event(),
        Some(Event::TypingChanged(change))
            if change.state == Typing::Done
                && change.reason == papo_core::TypingReason::TimedOut
    ));
    assert_eq!(session.state_generation(), 3);
}

#[test]
fn chat_messages_stop_typing_before_the_message_event() {
    for command in ["PRIVMSG", "NOTICE"] {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        session.handle_line(now, b"@+typing=active :alice!~a@cloak TAGMSG #papo");
        _ = session.poll_event();
        session.handle_line(
            now,
            format!(":alice!~a@cloak {command} #papo :hello from Alice").as_bytes(),
        );

        assert!(matches!(
            session.poll_event(),
            Some(Event::TypingChanged(change))
                if change.reason == papo_core::TypingReason::Messaged
        ));
        assert!(matches!(
            session.poll_event(),
            Some(Event::Line(line)) if line.as_ref().command_is(command)
        ));
        let conversation = conversation(&session, "#papo");
        assert_eq!(session.state().typing(&conversation).count(), 0);
    }
}

#[test]
fn part_and_kick_emit_departed_typing_before_departure() {
    for line in [
        ":alice!~a@cloak PART #papo :Leaving",
        ":ChanServ!service@services KICK #papo alice :Requested",
    ] {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        session.handle_line(now, b"@+typing=active :alice!~a@cloak TAGMSG #papo");
        _ = session.poll_event();
        session.handle_line(now, line.as_bytes());
        assert!(matches!(
            session.poll_event(),
            Some(Event::TypingChanged(change))
                if change.reason == papo_core::TypingReason::Departed
        ));
        assert!(matches!(
            session.poll_event(),
            Some(Event::Parted { .. } | Event::Kicked { .. })
        ));
    }
}

#[test]
fn quit_clears_typing_in_every_conversation() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    for channel in ["#papo", "#rust"] {
        session.handle_line(
            now,
            format!("@+typing=active :alice!~a@cloak TAGMSG {channel}").as_bytes(),
        );
        _ = session.poll_event();
    }
    session.handle_line(now, b":alice!~a@cloak QUIT :Client exited");

    for channel in ["#papo", "#rust"] {
        assert!(matches!(
            session.poll_event(),
            Some(Event::TypingChanged(change))
                if change.reason == papo_core::TypingReason::Departed
                    && change.conversation == conversation(&session, channel)
        ));
        assert_eq!(
            session
                .state()
                .typing(&conversation(&session, channel))
                .count(),
            0
        );
    }
    assert!(matches!(session.poll_event(), Some(Event::Quit { .. })));
}

#[test]
fn own_departures_and_disconnect_clear_typing_silently() {
    for departure in [
        Some(":papo!~p@localhost PART #papo :Leaving"),
        Some(":ChanServ!service@services KICK #papo papo :Requested"),
        None,
    ] {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(now, b"@+typing=active :alice!~a@cloak TAGMSG #papo");
        _ = session.poll_event();
        if let Some(line) = departure {
            session.handle_line(now, line.as_bytes());
            assert!(matches!(
                session.poll_event(),
                Some(Event::Parted { .. } | Event::Kicked { .. })
            ));
        } else {
            session.handle_disconnected(now);
        }
        assert!(!matches!(
            session.poll_event(),
            Some(Event::TypingChanged(_))
        ));
        let conversation = conversation(&session, "#papo");
        assert_eq!(session.state().typing(&conversation).count(), 0);
    }
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
