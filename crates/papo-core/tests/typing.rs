//! Transcript coverage for retained typing state.

mod support;

use std::time::{Duration, Instant};

use papo_core::{Config, Conversation, Event, Session};
use papo_proto::{Casemapping, Typing};
use support::Transcript;

fn conversation(session: &Session, target: &str) -> Conversation {
    session.state().parse_target(target).unwrap().1
}

#[test]
fn state_equality_includes_retained_typers() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    let mut equivalent = Session::new(Config::default());
    let before = session.state().clone();
    assert_eq!(session.state(), equivalent.state());

    session.handle_line(now, b"@+typing=active :alice!~a@cloak TAGMSG #papo");
    equivalent.handle_line(now, b"@+typing=active :alice!~a@cloak TAGMSG #papo");

    assert_ne!(&before, session.state());
    assert_eq!(session.state(), equivalent.state());
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
    let typing_generation = session.typing_generation();
    session.handle_line(
        now + std::time::Duration::from_secs(3),
        b"@+typing=active :ALICE!~a@cloak TAGMSG #papo",
    );
    assert_eq!(session.state_generation(), generation);
    assert_eq!(session.typing_generation(), typing_generation);
    assert_eq!(
        session.poll_timeout(),
        Some(now + std::time::Duration::from_secs(9))
    );
    _ = session.poll_event();

    let conversation = conversation(&session, "#papo");
    let typers: Vec<_> = session.state().typing(&conversation).collect();
    assert_eq!(typers.len(), 2);
    assert_eq!(typers[0].nick().as_ref(), "alice");
    assert!(!typers[0].is_paused());
    assert_eq!(typers[1].nick(), "bob");

    session.handle_timeout(now + std::time::Duration::from_secs(8));
    assert_eq!(session.state_generation(), generation);
    assert_eq!(session.typing_generation(), typing_generation);
    session.handle_timeout(now + std::time::Duration::from_secs(9));
    assert!(matches!(
        session.poll_event(),
        Some(Event::TypingChanged(change))
            if change.state == Typing::Done && change.reason == papo_core::TypingReason::TimedOut
    ));
    assert_eq!(session.state_generation(), generation);
    assert_eq!(session.typing_generation(), typing_generation + 1);
    let remaining: Vec<_> = session.state().typing(&conversation).collect();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].nick(), "bob");
}

#[test]
fn status_target_updates_typing_state_and_deadline() {
    Transcript::new(Config::default().nick("papo"))
        .connect()
        .server(":ergo.test 005 papo STATUSMSG=@+ :are supported by this server")
        .server("@+typing=active :alice!~a@cloak TAGMSG @#papo")
        .event(|event| {
            matches!(event, Event::TypingChanged(change) if change.return_address() == "@#papo")
        })
        .server("@+typing=paused :ALICE!~a@cloak TAGMSG @#papo")
        .event(|event| {
            matches!(event, Event::TypingChanged(change) if change.return_address() == "@#papo")
        })
        .inspect(|session| {
            let conversation = conversation(session, "#papo");
            let typers: Vec<_> = session.state().typing(&conversation).collect();
            assert_eq!(typers.len(), 1);
            assert!(typers[0].is_paused());
            assert_eq!(typers[0].status().map(papo_core::Prefix::symbol), Some('@'));
            assert_eq!(typers[0].return_address(), "@#papo");
        })
        .deadline(Some(Duration::from_secs(30)))
        .no_event()
        .ignore_rest();
}

#[test]
fn status_target_replaces_public_typing_state() {
    Transcript::new(Config::default().nick("papo"))
        .connect()
        .server(":ergo.test 005 papo STATUSMSG=@+ :are supported by this server")
        .server("@+typing=active :alice!~a@cloak TAGMSG #papo")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .server("@+typing=active :ALICE!~a@cloak TAGMSG @#papo")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .inspect(|session| {
            let conversation = conversation(session, "#papo");
            let typers: Vec<_> = session.state().typing(&conversation).collect();
            assert_eq!(typers.len(), 1);
            assert_eq!(typers[0].return_address(), "@#papo");
            assert_eq!(session.typing_generation(), 3);
        })
        .no_event()
        .ignore_rest();
}

#[test]
fn public_target_replaces_status_typing_state() {
    Transcript::new(Config::default().nick("papo"))
        .connect()
        .server(":ergo.test 005 papo STATUSMSG=@+ :are supported by this server")
        .server("@+typing=active :alice!~a@cloak TAGMSG @#papo")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .server("@+typing=active :ALICE!~a@cloak TAGMSG #papo")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .inspect(|session| {
            let conversation = conversation(session, "#papo");
            let typers: Vec<_> = session.state().typing(&conversation).collect();
            assert_eq!(typers.len(), 1);
            assert_eq!(typers[0].return_address(), "#papo");
            assert_eq!(session.typing_generation(), 3);
        })
        .no_event()
        .ignore_rest();
}

#[test]
fn typing_tag_on_chat_message_is_not_a_notification() {
    Transcript::new(Config::default().nick("papo"))
        .server(":ergo.test 005 papo STATUSMSG=@+ :are supported by this server")
        .server("@+typing=active :alice!~a@cloak PRIVMSG @#papo :status update")
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("PRIVMSG")))
        .inspect(|session| {
            let conversation = conversation(session, "#papo");
            assert_eq!(session.state().typing(&conversation).count(), 0);
        })
        .no_event();
}

#[test]
fn casemapping_change_refolds_and_deduplicates_typers() {
    Transcript::new(
        Config::default()
            .nick("papo")
            .casemapping(Casemapping::Ascii),
    )
    .connect()
    .server(":ergo.test 005 papo CASEMAPPING=ascii :are supported by this server")
    .server("@+typing=active :alice[bot]!~a@cloak TAGMSG #papo")
    .event(|event| matches!(event, Event::TypingChanged(_)))
    .server("@+typing=active :alice{bot}!~a@cloak TAGMSG #papo")
    .event(|event| matches!(event, Event::TypingChanged(_)))
    .inspect(|session| {
        let conversation = conversation(session, "#papo");
        assert_eq!(session.state().typing(&conversation).count(), 2);
    })
    .server(":ergo.test 005 papo CASEMAPPING=rfc1459 :are supported by this server")
    .event(|event| matches!(event, Event::CasemappingChanged { .. }))
    .inspect(|session| {
        let conversation = conversation(session, "#papo");
        let typers: Vec<_> = session.state().typing(&conversation).collect();
        assert_eq!(typers.len(), 1);
    })
    .advance(Duration::from_secs(6))
    .event(|event| {
        matches!(
            event,
            Event::TypingChanged(change)
                if change.state == Typing::Done
                    && change.reason == papo_core::TypingReason::TimedOut
        )
    })
    .no_event()
    .ignore_rest();
}

#[test]
fn paused_typing_expires_after_thirty_seconds() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_connected(now);
    session.handle_line(now, b"@+typing=paused :alice!~a@cloak TAGMSG #papo");
    _ = session.poll_event();
    let state_generation = session.state_generation();
    let typing_generation = session.typing_generation();
    assert_eq!(
        session.poll_timeout(),
        Some(now + std::time::Duration::from_secs(30))
    );

    session.handle_timeout(now + std::time::Duration::from_secs(29));
    assert_eq!(session.state_generation(), state_generation);
    assert_eq!(session.typing_generation(), typing_generation);
    session.handle_timeout(now + std::time::Duration::from_secs(30));
    assert!(matches!(
        session.poll_event(),
        Some(Event::TypingChanged(change))
            if change.state == Typing::Done
                && change.reason == papo_core::TypingReason::TimedOut
    ));
    assert_eq!(session.state_generation(), state_generation);
    assert_eq!(session.typing_generation(), typing_generation + 1);
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
fn part_scopes_other_typing_but_own_part_clears_the_channel() {
    Transcript::new(Config::default().nick("papo"))
        .server("@+typing=active :alice!~a@cloak TAGMSG #papo")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .server("@+typing=active :alice!~a@cloak TAGMSG #rust")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .server("@+typing=active :bob!~b@cloak TAGMSG #papo")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .server(":alice!~a@cloak PART #papo :Leaving")
        .event(|event| {
            matches!(
                event,
                Event::TypingChanged(change)
                    if change.actor == "alice!~a@cloak"
                        && change.reason == papo_core::TypingReason::Departed
            )
        })
        .event(|event| matches!(event, Event::Parted { .. }))
        .inspect(|session| {
            let papo = conversation(session, "#papo");
            let rust = conversation(session, "#rust");
            assert_eq!(session.state().typing(&papo).next().unwrap().nick(), "bob");
            assert_eq!(
                session.state().typing(&rust).next().unwrap().nick(),
                "alice"
            );
        })
        .server(":papo!~p@localhost PART #papo :Leaving")
        .event(|event| matches!(event, Event::Parted { .. }))
        .inspect(|session| {
            assert_eq!(
                session
                    .state()
                    .typing(&conversation(session, "#papo"))
                    .count(),
                0
            );
            assert_eq!(
                session
                    .state()
                    .typing(&conversation(session, "#rust"))
                    .count(),
                1
            );
        })
        .no_event();
}

#[test]
fn kick_only_clears_the_kicked_sender_in_that_channel() {
    Transcript::new(Config::default().nick("papo"))
        .server("@+typing=active :alice!~a@cloak TAGMSG #papo")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .server("@+typing=active :bob!~b@cloak TAGMSG #papo")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .server("@+typing=active :alice!~a@cloak TAGMSG #rust")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .server(":ChanServ!service@services KICK #papo alice :Requested")
        .event(|event| {
            matches!(
                event,
                Event::TypingChanged(change)
                    if change.actor == "alice!~a@cloak"
                        && change.reason == papo_core::TypingReason::Departed
            )
        })
        .event(|event| matches!(event, Event::Kicked { .. }))
        .inspect(|session| {
            let papo = conversation(session, "#papo");
            let rust = conversation(session, "#rust");
            assert_eq!(session.state().typing(&papo).next().unwrap().nick(), "bob");
            assert_eq!(
                session.state().typing(&rust).next().unwrap().nick(),
                "alice"
            );
        })
        .no_event();
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
fn nick_changes_carry_typing_and_its_deadline() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_connected(now);
    session.handle_line(now, b"@+typing=active :alice!~a@cloak TAGMSG #papo");
    _ = session.poll_event();
    session.handle_line(now, b":alice!~a@cloak NICK Alicia");
    assert!(matches!(
        session.poll_event(),
        Some(Event::NickChanged { .. })
    ));

    let conversation = conversation(&session, "#papo");
    let typer = session.state().typing(&conversation).next().unwrap();
    assert_eq!(typer.nick(), "Alicia");
    assert!(!typer.is_paused());
    assert_eq!(
        session.poll_timeout(),
        Some(now + std::time::Duration::from_secs(6))
    );
    session.handle_timeout(now + std::time::Duration::from_secs(6));
    assert!(matches!(
        session.poll_event(),
        Some(Event::TypingChanged(change)) if change.actor == "Alicia!~a@cloak"
    ));
}

#[test]
fn endless_typers_stay_within_literal_bounds() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    for index in 0..33 {
        session.handle_line(
            now,
            format!("@+typing=active :user{index:02}!u@cloak TAGMSG #papo").as_bytes(),
        );
        while session.poll_event().is_some() {}
    }
    let channel = conversation(&session, "#papo");
    let typers: Vec<_> = session.state().typing(&channel).collect();
    assert_eq!(typers.len(), 32);
    assert_eq!(typers[0].nick(), "user01");
    assert_eq!(typers[31].nick(), "user32");

    let mut session = Session::new(Config::default());
    for index in 0..513 {
        session.handle_line(
            now,
            format!("@+typing=active :user{index:03}!u@cloak TAGMSG #room{index:03}").as_bytes(),
        );
        while session.poll_event().is_some() {}
    }
    let retained: usize = (0..513)
        .map(|index| {
            let channel = conversation(&session, &format!("#room{index:03}"));
            session.state().typing(&channel).count()
        })
        .sum();
    assert_eq!(retained, 512);
    assert_eq!(
        session
            .state()
            .typing(&conversation(&session, "#room000"))
            .count(),
        0
    );
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
