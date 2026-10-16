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
fn recorded_ergo_multiline_batch_joins_text_and_metadata() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(
        now,
        b"@msgid=RKi9Cw;time=2026-10-15T12:00:01.000Z :alice!~a@cloak BATCH +m draft/multiline #papo",
    );
    session.handle_line(now, b"@batch=m :alice!~a@cloak PRIVMSG #papo :hello ");
    session.handle_line(
        now,
        b"@batch=m;draft/multiline-concat :alice!~a@cloak PRIVMSG #papo :world",
    );
    session.handle_line(now, b"@batch=m :alice!~a@cloak PRIVMSG #papo :next line");
    session.handle_line(now, b":alice!~a@cloak BATCH -m");

    let Some(Event::MultilineMessage(multiline)) = session.poll_event() else {
        panic!("expected multiline message")
    };
    let message = multiline.message.as_ref();
    assert_eq!(message.raw_source(), Some("alice!~a@cloak"));
    assert_eq!(message.param(0), Some("#papo"));
    assert_eq!(message.param(1), Some("hello world\nnext line"));
    assert_eq!(message.tag("msgid").as_deref(), Some("RKi9Cw"));
    assert_eq!(
        message.tag("time").as_deref(),
        Some("2026-10-15T12:00:01.000Z")
    );
}

#[test]
fn multiline_over_advertised_max_bytes_is_released() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().max_batch_lines(2));
    session.handle_line(now, b"CAP * LS :batch draft/multiline=max-bytes=10");
    session.handle_line(now, b"CAP * ACK :batch draft/multiline");
    session.handle_line(now, b"BATCH +m draft/multiline #papo");
    session.handle_line(now, b"@batch=m :alice PRIVMSG #papo :12345");
    session.handle_line(now, b"@batch=m :alice PRIVMSG #papo :67890");

    assert_eq!(message(&mut session).as_ref().param(1), Some("12345"));
    assert_eq!(message(&mut session).as_ref().param(1), Some("67890"));
    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason })
            if reference == "m"
                && batch_type == "draft/multiline"
                && reason == BatchAbandonReason::ByteLimit
    ));
}

fn multiline_session(now: Instant, max_bytes: usize, max_lines: usize) -> Session {
    let mut session = Session::new(Config::default().max_batch_lines(max_lines));
    session.handle_line(
        now,
        format!("CAP * LS :batch draft/multiline=max-bytes={max_bytes}").as_bytes(),
    );
    session.handle_line(now, b"CAP * ACK :batch draft/multiline");
    session
}

#[test]
fn multiline_accepts_exact_byte_boundaries() {
    let now = Instant::now();
    for (max_bytes, lines, expected) in [
        (5, vec![("12345", false)], "12345"),
        (10, vec![("12345", false), ("67890", true)], "1234567890"),
        (10, vec![("1234", false), ("56789", false)], "1234\n56789"),
    ] {
        let mut session = multiline_session(now, max_bytes, 10);
        session.handle_line(now, b"BATCH +m draft/multiline #papo");
        for (text, concat) in lines {
            let tag = if concat {
                "@batch=m;draft/multiline-concat"
            } else {
                "@batch=m"
            };
            session.handle_line(
                now,
                format!("{tag} :alice!~a@cloak PRIVMSG #papo :{text}").as_bytes(),
            );
        }
        session.handle_line(now, b"BATCH -m");
        assert!(matches!(
            session.poll_event(),
            Some(Event::MultilineMessage(multiline))
                if multiline.message.as_ref().param(1) == Some(expected)
        ));
    }
}

#[test]
fn multiline_accepts_the_line_limit_and_abandons_one_over() {
    let now = Instant::now();
    let mut exact = multiline_session(now, 100, 2);
    exact.handle_line(now, b"BATCH +m draft/multiline #papo");
    exact.handle_line(now, b"@batch=m :alice PRIVMSG #papo :one");
    exact.handle_line(now, b"@batch=m :alice PRIVMSG #papo :two");
    exact.handle_line(now, b"BATCH -m");
    assert!(matches!(
        exact.poll_event(),
        Some(Event::MultilineMessage(multiline))
            if multiline.message.as_ref().param(1) == Some("one\ntwo")
    ));

    let mut over = multiline_session(now, 100, 2);
    over.handle_line(now, b"BATCH +m draft/multiline #papo");
    for text in ["one", "two", "three"] {
        over.handle_line(
            now,
            format!("@batch=m :alice PRIVMSG #papo :{text}").as_bytes(),
        );
    }
    for expected in ["one", "two", "three"] {
        assert_eq!(message(&mut over).as_ref().param(1), Some(expected));
    }
    assert!(matches!(
        over.poll_event(),
        Some(Event::BatchAbandoned { reason, .. })
            if reason == BatchAbandonReason::LineLimit
    ));
}

#[test]
fn malformed_multiline_member_shapes_are_not_aggregated() {
    let now = Instant::now();
    for lines in [
        vec![
            "@batch=m :alice PRIVMSG #papo :one",
            "@batch=m :alice NOTICE #papo :two",
        ],
        vec![
            "@batch=m :alice PRIVMSG #papo :one",
            "@batch=m :alice PRIVMSG #other :two",
        ],
        vec!["@batch=m :alice PRIVMSG #papo"],
    ] {
        let mut session = multiline_session(now, 100, 10);
        session.handle_line(now, b"BATCH +m draft/multiline #papo");
        for line in lines {
            session.handle_line(now, line.as_bytes());
        }
        session.handle_line(now, b"BATCH -m");
        assert_eq!(session.poll_event(), None);
    }
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
fn duplicate_batch_reference_passes_through_without_replacing_the_batch() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b"BATCH +sync vendor/first");
    session.handle_line(now, b"BATCH +sync vendor/second");
    session.handle_line(now, b"@batch=sync :alice NOTICE papo :still first");

    let duplicate = message(&mut session);
    assert!(duplicate.as_ref().command_is("BATCH"));
    assert_eq!(duplicate.as_ref().param(0), Some("+sync"));
    assert_eq!(duplicate.as_ref().param(1), Some("vendor/second"));
    let member = message(&mut session);
    assert_eq!(member.batch_type(), Some("vendor/first"));
    assert_eq!(member.as_ref().param(1), Some("still first"));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn invalid_nested_batch_closes_pass_through() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b"BATCH +outer vendor/outer");
    session.handle_line(now, b"BATCH +other vendor/other");
    session.handle_line(now, b"@batch=outer BATCH +inner vendor/inner");
    session.handle_line(now, b"BATCH -outer");
    session.handle_line(now, b"@batch=other BATCH -inner");

    let outer = message(&mut session);
    assert_eq!(outer.as_ref().param(0), Some("-outer"));
    let inner = message(&mut session);
    assert_eq!(inner.batch_type(), Some("vendor/other"));
    assert_eq!(inner.as_ref().param(0), Some("-inner"));
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

    let Some(Event::History { target, items, end }) = session.poll_event() else {
        panic!("expected history")
    };
    assert_eq!(target, "#papo");
    assert!(end);
    assert_eq!(items.len(), 3);
    assert_eq!(
        items[0].message().unwrap().as_ref().tag("msgid").as_deref(),
        Some("RKi9Cw")
    );
    assert_eq!(
        items[1].message().unwrap().as_ref().param(1),
        Some("second")
    );
    assert_eq!(
        items[2].message().unwrap().as_ref().tag("time").as_deref(),
        Some("2026-10-15T12:00:03.000Z")
    );
}

#[test]
fn history_targets_require_the_command_and_subcommand_independently() {
    for member in [
        "@batch=targets :ergo.test NOTICE TARGETS #papo 2026-10-15T12:00:01.000Z",
        "@batch=targets :ergo.test CHATHISTORY LATEST #papo 2026-10-15T12:00:01.000Z",
    ] {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        session.handle_line(now, b":ergo.test BATCH +targets draft/chathistory-targets");
        session.handle_line(now, member.as_bytes());
        session.handle_line(now, b":ergo.test BATCH -targets");

        assert!(matches!(
            session.poll_event(),
            Some(Event::HistoryTargets(targets)) if targets.is_empty()
        ));
        assert_eq!(session.poll_event(), None);
    }
}

#[test]
fn empty_and_draft_history_types_complete() {
    for kind in ["chathistory", "draft/chathistory", "draft/event-playback"] {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        session.handle_line(now, format!("BATCH +h {kind} #papo").as_bytes());
        session.handle_line(now, b"BATCH -h");
        assert!(matches!(
            session.poll_event(),
            Some(Event::History { target, items, end: false })
                if target == "#papo" && items.is_empty()
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
        Some(Event::History { items, .. }) if items.len() == 1
    ));
}

#[test]
fn abandoned_outer_batch_flattens_completed_history_exactly() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .rate_limit(RateLimit::unlimited())
            .batch_timeout(Duration::from_secs(2)),
    );
    session.handle_connected(now);
    session.handle_line(now, b"BATCH +reply labeled-response");
    session.handle_line(now, b"@batch=reply BATCH +h chathistory #papo");
    session.handle_line(
        now,
        b"@batch=h;time=2026-10-15T12:00:01.000Z :alice!~a@cloak PRIVMSG #papo :from history",
    );
    session.handle_line(now, b"@batch=reply BATCH -h");
    session.handle_timeout(now + Duration::from_secs(2));

    let fallback = message(&mut session);
    assert_eq!(fallback.batch_type(), None);
    assert_eq!(
        fallback.into_message().to_line().unwrap(),
        "@batch=h;time=2026-10-15T12:00:01.000Z :alice!~a@cloak PRIVMSG #papo :from history\r\n"
    );
    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason })
            if reference == "reply"
                && batch_type == "labeled-response"
                && reason == BatchAbandonReason::Timeout
    ));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn outermost_aggregate_groups_its_commands_and_releases_others() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b"BATCH +split netsplit irc.example irc.remote");
    session.handle_line(now, b"@batch=split BATCH +h chathistory #papo");
    session.handle_line(now, b"@batch=h :alice!~a@remote.example QUIT :link lost");
    session.handle_line(now, b"@batch=h :irc.example NOTICE * :split metadata");
    session.handle_line(now, b"@batch=split BATCH -h");
    session.handle_line(now, b"BATCH -split");

    assert!(matches!(
        session.poll_event(),
        Some(Event::Netsplit { servers, quits })
            if servers == ["irc.example", "irc.remote"]
                && quits.len() == 1
                && quits[0].as_ref().param(0) == Some("link lost")
    ));
    let extra = message(&mut session);
    assert_eq!(extra.batch_type(), Some("netsplit"));
    assert!(extra.as_ref().command_is("NOTICE"));
    assert_eq!(extra.as_ref().param(1), Some("split metadata"));
    assert_eq!(session.poll_event(), None);
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
fn empty_membership_aggregates_are_observable() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b"BATCH +split netsplit irc.example irc.remote");
    session.handle_line(now, b"BATCH -split");
    session.handle_line(now, b"BATCH +join netjoin irc.example irc.remote");
    session.handle_line(now, b"BATCH -join");

    assert!(matches!(
        session.poll_event(),
        Some(Event::Netsplit { quits, .. }) if quits.is_empty()
    ));
    assert!(matches!(
        session.poll_event(),
        Some(Event::Netjoin { joins, .. }) if joins.is_empty()
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
fn unknown_batch_overflow_delivers_the_overflowing_line_before_abandonment() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().max_batch_lines(1));
    session.handle_line(now, b"BATCH +sync vendor/sync");
    session.handle_line(now, b"@batch=sync :alice NOTICE papo :within limit");
    assert_eq!(
        message(&mut session).as_ref().param(1),
        Some("within limit")
    );
    session.handle_line(now, b"@batch=sync :alice NOTICE papo :overflowing line");

    let overflow = message(&mut session);
    assert_eq!(overflow.batch_type(), Some("vendor/sync"));
    assert_eq!(overflow.as_ref().param(1), Some("overflowing line"));
    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason })
            if reference == "sync"
                && batch_type == "vendor/sync"
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

#[test]
fn nested_batches_expiring_together_abandon_only_the_outer_batch() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .rate_limit(RateLimit::unlimited())
            .batch_timeout(Duration::from_secs(2)),
    );
    session.handle_connected(now);
    session.handle_line(now, b"BATCH +outer labeled-response");
    session.handle_line(now, b"@batch=outer BATCH +h chathistory #papo");
    session.handle_line(now, b"@batch=h :alice PRIVMSG #papo :nested fallback");
    session.handle_timeout(now + Duration::from_secs(2));

    let fallback = message(&mut session);
    assert_eq!(fallback.batch_type(), Some("chathistory"));
    assert_eq!(fallback.as_ref().param(1), Some("nested fallback"));
    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason })
            if reference == "outer"
                && batch_type == "labeled-response"
                && reason == BatchAbandonReason::Timeout
    ));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn only_live_membership_aggregates_change_channel_state() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    session.handle_line(now, b":alice!~a@remote.example JOIN #papo");
    while session.poll_event().is_some() {}
    let generation = session.state_generation();

    session.handle_line(now, b"BATCH +history chathistory #papo");
    session.handle_line(
        now,
        b"@batch=history :bob!~b@remote.example JOIN #papo bob :Bob Example",
    );
    session.handle_line(now, b"BATCH -history");
    assert!(matches!(session.poll_event(), Some(Event::History { .. })));
    assert_eq!(session.state_generation(), generation);
    assert!(
        session
            .state()
            .channel("#papo")
            .unwrap()
            .member("bob")
            .is_none()
    );

    session.handle_line(now, b"BATCH +split netsplit irc.example irc.remote");
    session.handle_line(
        now,
        b"@batch=split :alice!~a@remote.example QUIT :irc.example irc.remote",
    );
    session.handle_line(now, b"BATCH -split");
    assert!(matches!(session.poll_event(), Some(Event::Netsplit { .. })));
    assert_eq!(session.state_generation(), generation + 1);
    assert!(
        session
            .state()
            .channel("#papo")
            .unwrap()
            .member("alice")
            .is_none()
    );

    session.handle_line(now, b"BATCH +join netjoin irc.example irc.remote");
    session.handle_line(
        now,
        b"@batch=join :alice!~a@remote.example JOIN #papo alice :Alice Example",
    );
    session.handle_line(now, b"BATCH -join");
    assert!(matches!(session.poll_event(), Some(Event::Netjoin { .. })));
    assert_eq!(session.state_generation(), generation + 2);
    assert!(
        session
            .state()
            .channel("#papo")
            .unwrap()
            .member("alice")
            .is_some()
    );
    assert_eq!(session.poll_event(), None);
}

#[test]
fn netjoin_for_our_nick_honors_no_implicit_names() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().wanted_caps(["no-implicit-names"]));
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :draft/no-implicit-names");
    session.handle_line(now, b"CAP * ACK :draft/no-implicit-names");
    session.handle_line(now, b"BATCH +join netjoin irc.example irc.remote");
    session.handle_line(now, b"@batch=join :papo!papo@127.0.0.1 JOIN #papo");
    session.handle_line(now, b"BATCH -join");

    assert!(matches!(
        session.poll_event(),
        Some(Event::Netjoin { joins, .. }) if joins.len() == 1
    ));
    assert!(session.state().channel("#papo").unwrap().is_synced());
    assert_eq!(session.poll_event(), None);
}
