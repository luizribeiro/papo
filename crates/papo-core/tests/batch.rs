//! Transcript coverage for `IRCv3` batch routing.

use std::time::{Duration, Instant};

use papo_core::{BatchAbandonReason, Config, Event, RateLimit, Session};

fn message(session: &mut Session) -> papo_core::ReceivedMessage {
    let Some(Event::Message(message)) = session.poll_event() else {
        panic!("expected a message event")
    };
    message
}

#[test]
fn unknown_nested_and_interleaved_batches_flow_with_their_types() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b":ergo.test BATCH +sync vendor/sync #papo");
    session.handle_line(now, b":ergo.test BATCH +other vendor/other");
    session.handle_line(now, b"@batch=sync :ergo.test BATCH +nested vendor/nested");
    session.handle_line(
        now,
        b"@batch=nested :alice!~a@cloak PRIVMSG #papo :nested line",
    );
    session.handle_line(
        now,
        b"@batch=other :bob!~b@cloak NOTICE papo :interleaved line",
    );

    let nested = message(&mut session);
    assert_eq!(nested.batch_type(), Some("vendor/nested"));
    assert_eq!(nested.as_ref().param(1), Some("nested line"));
    let other = message(&mut session);
    assert_eq!(other.batch_type(), Some("vendor/other"));
    assert_eq!(other.as_ref().param(1), Some("interleaved line"));

    session.handle_line(now, b"@batch=sync :ergo.test BATCH -nested");
    session.handle_line(now, b":ergo.test BATCH -sync");
    session.handle_line(now, b":ergo.test BATCH -other");
    assert_eq!(session.poll_event(), None);
}

#[test]
fn malformed_batch_commands_pass_through() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b":ergo.test BATCH +bad_ref vendor/sync");
    session.handle_line(now, b":ergo.test BATCH -missing");

    assert!(message(&mut session).as_ref().command_is("BATCH"));
    assert!(message(&mut session).as_ref().command_is("BATCH"));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn disconnect_forgets_open_batches() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b":ergo.test BATCH +sync vendor/sync");
    session.handle_disconnected(now);
    session.handle_connected(now);
    session.handle_line(
        now,
        b"@batch=sync :alice!~a@cloak PRIVMSG #papo :after reconnect",
    );

    assert_eq!(message(&mut session).batch_type(), None);
}

#[test]
fn recorded_ergo_latest_batch_becomes_history() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(
        now,
        b"@draft/chathistory-end :ergo.test BATCH +c chathistory #papo",
    );
    for line in [
        "@batch=c;time=2026-10-15T12:00:01.000Z;msgid=RKi9Cw :alice!~a@cloak PRIVMSG #papo :first",
        "@batch=c;time=2026-10-15T12:00:02.000Z;msgid=8sKY2A :bob!~b@cloak PRIVMSG #papo :second",
        "@batch=c;time=2026-10-15T12:00:03.000Z;msgid=T1pQ6g :alice!~a@cloak PRIVMSG #papo :third",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    assert_eq!(session.poll_event(), None);
    session.handle_line(now, b":ergo.test BATCH -c");

    let Some(Event::History {
        target,
        messages,
        end,
    }) = session.poll_event()
    else {
        panic!("expected history")
    };
    assert_eq!(target, "#papo");
    assert!(end);
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].as_ref().tag("msgid").as_deref(), Some("RKi9Cw"));
    assert_eq!(messages[1].as_ref().param(1), Some("second"));
    assert_eq!(
        messages[2].as_ref().tag("time").as_deref(),
        Some("2026-10-15T12:00:03.000Z")
    );
}

#[test]
fn empty_and_draft_history_types_complete() {
    for kind in [
        "chathistory",
        "draft/chathistory",
        "draft/chathistory-targets",
        "draft/event-playback",
    ] {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        session.handle_line(now, format!("BATCH +h {kind} #papo").as_bytes());
        session.handle_line(now, b"BATCH -h");
        assert!(matches!(
            session.poll_event(),
            Some(Event::History { target, messages, end: false })
                if target == "#papo" && messages.is_empty()
        ));
    }
}

#[test]
fn nested_history_waits_for_the_outer_batch() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b"BATCH +reply labeled-response");
    session.handle_line(now, b"@batch=reply BATCH +h chathistory #papo");
    session.handle_line(now, b"@batch=h :alice!~a@cloak PRIVMSG #papo :from history");
    session.handle_line(now, b"@batch=reply BATCH -h");
    assert_eq!(session.poll_event(), None);
    session.handle_line(now, b"BATCH -reply");
    assert!(matches!(
        session.poll_event(),
        Some(Event::History { messages, .. }) if messages.len() == 1
    ));
}

#[test]
fn interleaved_histories_complete_independently() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b"BATCH +a chathistory #a");
    session.handle_line(now, b"BATCH +b chathistory #b");
    session.handle_line(now, b"@batch=a :alice PRIVMSG #a :one");
    session.handle_line(now, b"@batch=b :bob PRIVMSG #b :two");
    session.handle_line(now, b"BATCH -b");
    session.handle_line(now, b"BATCH -a");
    assert!(matches!(session.poll_event(), Some(Event::History { target, .. }) if target == "#b"));
    assert!(matches!(session.poll_event(), Some(Event::History { target, .. }) if target == "#a"));
}

#[test]
fn netsplit_followed_by_netjoin_becomes_two_aggregate_events() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(
        now,
        b":irc.example BATCH +split netsplit irc.example irc.remote",
    );
    session.handle_line(
        now,
        b"@batch=split :alice!~a@remote.example QUIT :irc.example irc.remote",
    );
    session.handle_line(
        now,
        b"@batch=split :bob!~b@remote.example QUIT :irc.example irc.remote",
    );
    session.handle_line(now, b"@batch=split :irc.example NOTICE * :link lost");
    session.handle_line(now, b":irc.example BATCH -split");
    session.handle_line(
        now,
        b":irc.example BATCH +join netjoin irc.example irc.remote",
    );
    session.handle_line(
        now,
        b"@batch=join :alice!~a@remote.example JOIN #papo alice :Alice Example",
    );
    session.handle_line(
        now,
        b"@batch=join :bob!~b@remote.example JOIN #papo bob :Bob Example",
    );
    session.handle_line(now, b":irc.example BATCH -join");

    assert!(matches!(
        session.poll_event(),
        Some(Event::Netsplit { servers, quits })
            if servers == ["irc.example", "irc.remote"] && quits.len() == 2
    ));
    assert!(matches!(
        session.poll_event(),
        Some(Event::Netjoin { servers, joins })
            if servers == ["irc.example", "irc.remote"] && joins.len() == 2
    ));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn rejects_a_batch_beyond_the_configured_open_limit() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().max_open_batches(2));
    session.handle_line(now, b"BATCH +a vendor/one");
    session.handle_line(now, b"BATCH +b vendor/two");
    session.handle_line(now, b"BATCH +c chathistory #papo");

    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason })
            if reference == "c"
                && batch_type == "chathistory"
                && reason == BatchAbandonReason::OpenBatchLimit
    ));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn abandons_history_on_the_first_line_beyond_its_limit() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().max_batch_lines(2));
    session.handle_line(now, b"BATCH +h chathistory #papo");
    session.handle_line(now, b"@batch=h :alice PRIVMSG #papo :one");
    session.handle_line(now, b"@batch=h :alice PRIVMSG #papo :two");
    assert_eq!(session.poll_event(), None);
    session.handle_line(now, b"@batch=h :alice PRIVMSG #papo :three");

    for expected in ["one", "two", "three"] {
        let fallback = message(&mut session);
        assert_eq!(fallback.batch_type(), Some("chathistory"));
        assert_eq!(fallback.as_ref().param(1), Some(expected));
    }
    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason })
            if reference == "h"
                && batch_type == "chathistory"
                && reason == BatchAbandonReason::LineLimit
    ));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn abandons_an_unclosed_batch_at_its_deadline() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .rate_limit(RateLimit::unlimited())
            .batch_timeout(Duration::from_secs(2)),
    );
    session.handle_connected(now);
    session.handle_line(now, b"BATCH +h chathistory #papo");
    session.handle_line(now, b"@batch=h :alice PRIVMSG #papo :remembered");
    assert_eq!(session.poll_timeout(), Some(now + Duration::from_secs(2)));
    session.handle_timeout(now + Duration::from_secs(1));
    assert_eq!(session.poll_event(), None);
    session.handle_timeout(now + Duration::from_secs(2));

    let fallback = message(&mut session);
    assert_eq!(fallback.as_ref().param(1), Some("remembered"));
    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason })
            if reference == "h"
                && batch_type == "chathistory"
                && reason == BatchAbandonReason::Timeout
    ));
}
