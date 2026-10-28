//! Transcript coverage for `IRCv3` batch routing.

use std::time::{Duration, Instant};

use papo_core::{
    Authorship, BatchAbandonReason, Config, Conversation, Event, HistoryItem, RateLimit, Session,
};

fn line(session: &mut Session) -> papo_core::ReceivedLine {
    let Some(Event::Line(line)) = session.poll_event() else {
        panic!("expected a line event")
    };
    line
}

#[test]
fn unrequested_soju_search_results_keep_their_stable_line_shape() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b":soju.example BATCH +search-42 soju.im/search");
    session.handle_line(now, b"@batch=search-42 :alice PRIVMSG #papo :first result");
    assert_eq!(line(&mut session).as_ref().param(1), Some("first result"));
    session.handle_line(now, b":soju.example BATCH -search-42");
    assert_eq!(session.poll_event(), None);
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
fn multiline_classification_survives_a_nick_change_before_polling() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo-driver"));
    session.handle_line(
        now,
        b":bob!~b@localhost BATCH +m draft/multiline papo-driver",
    );
    session.handle_line(
        now,
        b"@batch=m :bob!~b@localhost PRIVMSG papo-driver :first line",
    );
    session.handle_line(
        now,
        b"@batch=m :bob!~b@localhost PRIVMSG papo-driver :second line",
    );
    session.handle_line(now, b":bob!~b@localhost BATCH -m");
    session.handle_line(now, b":papo-driver!~papo@localhost NICK papo-away");

    let Some(Event::MultilineMessage(multiline)) = session.poll_event() else {
        panic!("expected multiline message")
    };
    assert!(matches!(
        multiline.message.conversation(),
        Some(Conversation::Private(nick)) if nick == "bob"
    ));
    assert_eq!(multiline.message.authorship(), Authorship::Other);
}

#[test]
fn multiline_over_advertised_max_bytes_is_released() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().max_batch_lines(2));
    session.handle_line(now, b"CAP * LS :batch draft/multiline=max-bytes=512");
    session.handle_line(now, b"CAP * ACK :batch draft/multiline");
    session.handle_line(now, b"BATCH +m draft/multiline #papo");
    let first = "a".repeat(256);
    let second = "b".repeat(256);
    session.handle_line(
        now,
        format!("@batch=m :alice PRIVMSG #papo :{first}").as_bytes(),
    );
    session.handle_line(
        now,
        format!("@batch=m :alice PRIVMSG #papo :{second}").as_bytes(),
    );

    assert_eq!(line(&mut session).as_ref().param(1), Some(first.as_str()));
    assert_eq!(line(&mut session).as_ref().param(1), Some(second.as_str()));
    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason, .. })
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
    for (case, (lines, expected)) in [
        (
            vec![
                ("a".repeat(170), false),
                ("b".repeat(170), false),
                ("c".repeat(170), false),
            ],
            format!(
                "{}\n{}\n{}",
                "a".repeat(170),
                "b".repeat(170),
                "c".repeat(170)
            ),
        ),
        (
            vec![("a".repeat(256), false), ("b".repeat(256), true)],
            format!("{}{}", "a".repeat(256), "b".repeat(256)),
        ),
        (
            vec![("a".repeat(255), false), ("b".repeat(256), false)],
            format!("{}\n{}", "a".repeat(255), "b".repeat(256)),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut session = multiline_session(now, 512, 10);
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
        let event = session.poll_event();
        assert!(
            matches!(
                event,
                Some(Event::MultilineMessage(ref multiline))
                    if multiline.message.as_ref().param(1) == Some(expected.as_str())
            ),
            "boundary case {case}: {event:?}"
        );
    }
}

#[test]
fn multiline_accepts_the_line_limit_and_abandons_one_over() {
    let now = Instant::now();
    let mut exact = multiline_session(now, 512, 2);
    exact.handle_line(now, b"BATCH +m draft/multiline #papo");
    exact.handle_line(now, b"@batch=m :alice PRIVMSG #papo :one");
    exact.handle_line(now, b"@batch=m :alice PRIVMSG #papo :two");
    exact.handle_line(now, b"BATCH -m");
    assert!(matches!(
        exact.poll_event(),
        Some(Event::MultilineMessage(multiline))
            if multiline.message.as_ref().param(1) == Some("one\ntwo")
    ));

    let mut over = multiline_session(now, 512, 2);
    over.handle_line(now, b"BATCH +m draft/multiline #papo");
    for text in ["one", "two", "three"] {
        over.handle_line(
            now,
            format!("@batch=m :alice PRIVMSG #papo :{text}").as_bytes(),
        );
    }
    for expected in ["one", "two", "three"] {
        assert_eq!(line(&mut over).as_ref().param(1), Some(expected));
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
        let mut session = multiline_session(now, 512, 10);
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

    let nested = line(&mut session);
    assert_eq!(nested.batch_type(), Some("vendor/nested"));
    assert_eq!(nested.as_ref().param(1), Some("nested line"));
    let other = line(&mut session);
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

    assert!(line(&mut session).as_ref().command_is("BATCH"));
    assert!(line(&mut session).as_ref().command_is("BATCH"));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn duplicate_batch_reference_passes_through_without_replacing_the_batch() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b"BATCH +sync vendor/first");
    session.handle_line(now, b"BATCH +sync vendor/second");
    session.handle_line(now, b"@batch=sync :alice NOTICE papo :still first");

    let duplicate = line(&mut session);
    assert!(duplicate.as_ref().command_is("BATCH"));
    assert_eq!(duplicate.as_ref().param(0), Some("+sync"));
    assert_eq!(duplicate.as_ref().param(1), Some("vendor/second"));
    let member = line(&mut session);
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

    let outer = line(&mut session);
    assert_eq!(outer.as_ref().param(0), Some("-outer"));
    let inner = line(&mut session);
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

    assert_eq!(line(&mut session).batch_type(), None);
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
        conversation,
        items,
        end,
        ..
    }) = session.poll_event()
    else {
        panic!("expected history")
    };
    assert!(matches!(conversation, Conversation::Channel(channel) if channel == "#papo"));
    assert!(end);
    assert_eq!(items.len(), 3);
    assert_eq!(
        items[0]
            .as_message()
            .unwrap()
            .as_ref()
            .tag("msgid")
            .as_deref(),
        Some("RKi9Cw")
    );
    assert_eq!(
        items[1].as_message().unwrap().as_ref().param(1),
        Some("second")
    );
    assert_eq!(
        items[2]
            .as_message()
            .unwrap()
            .as_ref()
            .tag("time")
            .as_deref(),
        Some("2026-10-15T12:00:03.000Z")
    );
}

#[test]
fn classifies_event_playback_items_in_wire_order() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b":ergo.test BATCH +play draft/event-playback #papo");
    for line in [
        "@batch=play;msgid=RKi9Cw :alice!~a@cloak PRIVMSG #papo :hello",
        "@batch=play;+draft/react=👍;+reply=RKi9Cw;draft/chathistory-context :bob!~b@cloak TAGMSG #papo",
        "@batch=play :alice!~a@cloak REDACT #papo RKi9Cw :Removed by author",
        "@batch=play;+typing=active :bob!~b@cloak TAGMSG #papo",
        "@batch=play :carol!~c@cloak JOIN #papo carol :Carol Example",
        "@batch=play :carol!~c@cloak PART #papo :Leaving",
        "@batch=play :carol!~c@cloak QUIT :Client exited",
        "@batch=play :alice!~a@cloak KICK #papo bob :Flooding",
        "@batch=play :bob!~b@cloak NICK robert",
        "@batch=play :alice!~a@cloak TOPIC #papo :Rust 1.90 is out",
        "@batch=play :alice!~a@cloak MODE #papo +v robert",
        "@batch=play :NickServ!service@services NOTICE #papo :Account registered",
        "@batch=play :ergo.test WALLOPS :Network maintenance begins soon",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    session.handle_line(now, b":ergo.test BATCH -play");

    let Some(Event::History { items, .. }) = session.poll_event() else {
        panic!("expected event playback history")
    };
    assert!(matches!(
        &items[..],
        [
            HistoryItem::Message(_),
            HistoryItem::Reacted(_),
            HistoryItem::Redacted(_),
            HistoryItem::TagMsg(_),
            HistoryItem::Joined(_),
            HistoryItem::Parted(_),
            HistoryItem::Quit(_),
            HistoryItem::Kicked(_),
            HistoryItem::NickChanged(_),
            HistoryItem::TopicChanged(_),
            HistoryItem::ModeChanged(_),
            HistoryItem::Message(_),
            HistoryItem::Other(_)
        ]
    ));
    assert!(items[0].as_message().is_some());
    assert!(items[1].as_reacted().is_some());
    assert!(items[2].as_redacted().is_some());
    assert!(items[3].as_tag_msg().is_some());
    assert!(items[4].as_joined().is_some());
    assert!(items[5].as_parted().is_some());
    assert!(items[6].as_quit().is_some());
    assert!(items[7].as_kicked().is_some());
    assert!(items[8].as_nick_changed().is_some());
    assert!(items[9].as_topic_changed().is_some());
    assert!(items[10].as_mode_changed().is_some());
    assert!(items[12].as_other().is_some());
    assert!(items[0].as_other().is_none());
    assert!(items[1].clone().into_line().context);
    assert_eq!(items.iter().filter_map(HistoryItem::as_message).count(), 2);
    let message = items[0].clone().into_message().unwrap();
    assert_eq!(message.as_ref().param(1), Some("hello"));
    assert!(items[4].clone().into_message().is_none());
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
            Some(Event::History { conversation: Conversation::Channel(channel), items, end: true, .. })
                if channel == "#papo" && items.is_empty()
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

    let fallback = line(&mut session);
    assert_eq!(fallback.batch_type(), None);
    assert_eq!(
        fallback.into_line().to_line().unwrap(),
        "@batch=h;time=2026-10-15T12:00:01.000Z :alice!~a@cloak PRIVMSG #papo :from history\r\n"
    );
    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason, .. })
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
        Some(Event::Quit { nick, netsplit: Some(split), authorship, .. })
            if nick == "alice" && split.batch == "split" && authorship == Authorship::Other
    ));
    let extra = line(&mut session);
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
    assert!(
        matches!(session.poll_event(), Some(Event::History { conversation: Conversation::Channel(channel), .. }) if channel == "#b")
    );
    assert!(
        matches!(session.poll_event(), Some(Event::History { conversation: Conversation::Channel(channel), .. }) if channel == "#a")
    );
}

#[test]
fn netsplit_followed_by_netjoin_becomes_individual_events() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(
        now,
        b":irc.example BATCH +split netsplit irc.example irc.remote",
    );
    session.handle_line(
        now,
        b"@batch=split :papo!~papo@remote.example QUIT :irc.example irc.remote",
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
        b"@batch=join :papo!~papo@remote.example JOIN #papo papo :Papo Client",
    );
    session.handle_line(
        now,
        b"@batch=join :bob!~b@remote.example JOIN #papo bob :Bob Example",
    );
    session.handle_line(now, b":irc.example BATCH -join");

    for (nick, authorship) in [("papo", Authorship::Own), ("bob", Authorship::Other)] {
        assert!(matches!(
            session.poll_event(),
            Some(Event::Quit { nick: actual, netsplit: Some(split), authorship: actual_authorship, .. })
                if actual == nick && actual_authorship == authorship && split.batch == "split"
        ));
    }
    for (nick, authorship) in [("papo", Authorship::Own), ("bob", Authorship::Other)] {
        assert!(matches!(
            session.poll_event(),
            Some(Event::Joined { nick: actual, netjoin: Some(split), authorship: actual_authorship, .. })
                if actual == nick && actual_authorship == authorship && split.batch == "join"
        ));
    }
    assert_eq!(session.poll_event(), None);
}

#[test]
fn empty_membership_batches_emit_nothing() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b"BATCH +split netsplit irc.example irc.remote");
    session.handle_line(now, b"BATCH -split");
    session.handle_line(now, b"BATCH +join netjoin irc.example irc.remote");
    session.handle_line(now, b"BATCH -join");

    assert_eq!(session.poll_event(), None);
}

#[test]
fn recorded_solanum_netsplit_emits_a_marked_quit() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    for line in [
        ":papo!~papo@localhost JOIN #papo",
        ":ChanServ!ChanServ@services.int JOIN #papo",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    while session.poll_event().is_some() {}

    session.handle_line(
        now,
        b":irc.papo.test BATCH +uoiRnDa1CRdpNmJ netsplit irc.papo.test services.papo.test",
    );
    session.handle_line(
        now,
        b"@batch=uoiRnDa1CRdpNmJ :ChanServ!ChanServ@services.int QUIT :irc.papo.test services.papo.test",
    );
    session.handle_line(now, b":irc.papo.test BATCH -uoiRnDa1CRdpNmJ");

    let Some(Event::Quit {
        nick,
        channels,
        netsplit: Some(split),
        ..
    }) = session.poll_event()
    else {
        panic!("expected a netsplit quit")
    };
    assert_eq!(nick, "ChanServ");
    assert_eq!(split.batch, "uoiRnDa1CRdpNmJ");
    assert_eq!(split.first, "irc.papo.test");
    assert_eq!(split.second, "services.papo.test");
    assert_eq!(channels.len(), 1);
    assert!(channels.iter().any(|channel| channel == "#papo"));
    assert!(session.state().user("ChanServ").is_none());
}

#[test]
fn equal_server_pairs_keep_distinct_batch_identities() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    for (reference, nick) in [("split-a", "alice"), ("split-b", "bob")] {
        session.handle_line(
            now,
            format!("BATCH +{reference} netsplit irc.papo.test services.papo.test").as_bytes(),
        );
        session.handle_line(
            now,
            format!(
                "@batch={reference} :{nick}!~user@services.int QUIT :irc.papo.test services.papo.test"
            )
            .as_bytes(),
        );
        session.handle_line(now, format!("BATCH -{reference}").as_bytes());
    }
    session.handle_line(
        now,
        b":mallory!~user@localhost QUIT :irc.papo.test services.papo.test",
    );

    for expected in ["split-a", "split-b"] {
        assert!(matches!(
            session.poll_event(),
            Some(Event::Quit { netsplit: Some(split), .. }) if split.batch == expected
        ));
    }
    assert!(matches!(
        session.poll_event(),
        Some(Event::Quit { netsplit: None, .. })
    ));
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
        Some(Event::BatchAbandoned { reference, batch_type, reason, .. })
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
    session.handle_line(now, b"BATCH +h draft/event-playback #papo");
    session.handle_line(
        now,
        b"@batch=h :alice!~a@cloak JOIN #papo alice :Alice Example",
    );
    session.handle_line(now, b"@batch=h :alice!~a@cloak PART #papo :Leaving");
    assert_eq!(session.poll_event(), None);
    session.handle_line(now, b"@batch=h :bob!~b@cloak MODE #papo +v bob");

    assert!(matches!(
        session.poll_event(),
        Some(Event::History { items, end: false, .. })
            if matches!(items.as_slice(), [
                HistoryItem::Joined(_),
                HistoryItem::Parted(_),
                HistoryItem::ModeChanged(_),
            ])
    ));
    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason, .. })
            if reference == "h"
                && batch_type == "draft/event-playback"
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
    assert_eq!(line(&mut session).as_ref().param(1), Some("within limit"));
    session.handle_line(now, b"@batch=sync :alice NOTICE papo :overflowing line");

    let overflow = line(&mut session);
    assert_eq!(overflow.batch_type(), Some("vendor/sync"));
    assert_eq!(overflow.as_ref().param(1), Some("overflowing line"));
    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason, .. })
            if reference == "sync"
                && batch_type == "vendor/sync"
                && reason == BatchAbandonReason::LineLimit
    ));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn repeated_completed_children_count_toward_the_parent_limit() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().max_batch_lines(2));
    session.handle_line(now, b":ergo.test BATCH +outer vendor/sync");
    for _ in 0..3 {
        session.handle_line(now, b"@batch=outer :ergo.test BATCH +inner metadata");
        session.handle_line(now, b"@batch=outer :ergo.test BATCH -inner");
    }

    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason, .. })
            if reference == "outer"
                && batch_type == "vendor/sync"
                && reason == BatchAbandonReason::LineLimit
    ));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn late_members_of_every_abandoned_batch_stay_out_of_live_state() {
    let now = Instant::now();
    for trigger in ["line-limit", "timeout", "open-limit"] {
        let config = match trigger {
            "line-limit" => Config::default().max_batch_lines(2),
            "timeout" => Config::default().batch_timeout(Duration::from_secs(1)),
            "open-limit" => Config::default().max_open_batches(1),
            _ => unreachable!(),
        };
        let mut session = Session::new(config.rate_limit(RateLimit::unlimited()));
        session.handle_connected(now);
        while session.poll_transmit().is_some() {}
        session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
        while session.poll_event().is_some() {}
        while session.poll_transmit().is_some() {}

        match trigger {
            "line-limit" => {
                session.handle_line(now, b"BATCH +h draft/event-playback #papo");
                for text in ["first replay", "second replay", "third replay"] {
                    session.handle_line(
                        now,
                        format!("@batch=h :alice PRIVMSG #papo :{text}").as_bytes(),
                    );
                }
            }
            "timeout" => {
                session.handle_line(now, b"BATCH +h draft/event-playback #papo");
                session.handle_timeout(now + Duration::from_secs(1));
            }
            "open-limit" => {
                session.handle_line(now, b"BATCH +blocker vendor/sync");
                session.handle_line(now, b"BATCH +h draft/event-playback #papo");
            }
            _ => unreachable!(),
        }
        while session.poll_event().is_some() {}

        session.handle_line(
            now,
            b"@batch=h :alice!~alice@staff.example PRIVMSG papo :\x01VERSION\x01",
        );
        session.handle_line(
            now,
            b"@batch=h :papo!~papo@localhost PART #papo :old playback",
        );

        assert_eq!(session.poll_transmit(), None, "{trigger}");
        assert!(session.state().channel("#papo").is_some(), "{trigger}");
        let late: Vec<_> = std::iter::from_fn(|| session.poll_event()).collect();
        assert_eq!(
            late.iter()
                .filter(|event| matches!(event, Event::Line(line)
                    if line.batch_type() == Some("draft/event-playback")))
                .count(),
            2,
            "{trigger}: {late:?}"
        );
        assert!(
            late.iter()
                .all(|event| !matches!(event, Event::Parted { .. })),
            "{trigger}: {late:?}"
        );
    }
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

    assert!(matches!(
        session.poll_event(),
        Some(Event::History { items, end: false, .. })
            if items.first().and_then(HistoryItem::as_message)
                .and_then(|item| item.line.as_ref().param(1)) == Some("remembered")
    ));
    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason, .. })
            if reference == "h"
                && batch_type == "chathistory"
                && reason == BatchAbandonReason::Timeout
    ));
}

#[test]
fn simultaneous_batch_timeouts_have_stable_order() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().batch_timeout(Duration::from_secs(2)));
    for reference in [
        "zulu", "echo", "hotel", "alpha", "golf", "bravo", "foxtrot", "charlie",
    ] {
        session.handle_line(
            now,
            format!("BATCH +{reference} vendor/{reference}").as_bytes(),
        );
    }

    session.handle_timeout(now + Duration::from_secs(2));

    let abandoned: Vec<_> = std::iter::from_fn(|| session.poll_event())
        .filter_map(|event| match event {
            Event::BatchAbandoned { reference, .. } => Some(reference),
            _ => None,
        })
        .collect();
    assert_eq!(
        abandoned,
        [
            "alpha", "bravo", "charlie", "echo", "foxtrot", "golf", "hotel", "zulu"
        ]
    );
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

    let fallback = line(&mut session);
    assert_eq!(fallback.batch_type(), Some("chathistory"));
    assert_eq!(fallback.as_ref().param(1), Some("nested fallback"));
    assert!(matches!(
        session.poll_event(),
        Some(Event::BatchAbandoned { reference, batch_type, reason, .. })
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
    assert!(matches!(
        session.poll_event(),
        Some(Event::Quit {
            netsplit: Some(_),
            ..
        })
    ));
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
    assert!(matches!(
        session.poll_event(),
        Some(Event::Joined {
            netjoin: Some(_),
            ..
        })
    ));
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
fn history_page_events_leave_channel_state_untouched() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    for line in [
        ":papo!~papo@localhost JOIN #papo",
        ":alice!~a@remote.example JOIN #papo",
        ":operator!~o@services.example MODE #papo +m",
        ":alice!~a@remote.example TOPIC #papo :Current topic",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    while session.poll_event().is_some() {}
    let generation = session.state_generation();

    session.handle_line(now, b":ergo.test BATCH +history chathistory #papo");
    for line in [
        "@batch=history :bob!~b@remote.example JOIN #papo bob :Bob Example",
        "@batch=history :alice!~a@remote.example PART #papo :Earlier part",
        "@batch=history :operator!~o@services.example KICK #papo papo :Earlier kick",
        "@batch=history :operator!~o@services.example MODE #papo +k history-key-4286",
        "@batch=history :operator!~o@services.example TOPIC #papo :Earlier topic",
        "@batch=history :alice!~a@remote.example NICK alice-away",
    ] {
        session.handle_line(now, line.as_bytes());
        assert_eq!(session.state_generation(), generation);
    }
    session.handle_line(now, b":ergo.test BATCH -history");

    let Some(Event::History { items, .. }) = session.poll_event() else {
        panic!("expected history page")
    };
    assert!(matches!(
        &items[..],
        [
            HistoryItem::Joined(_),
            HistoryItem::Parted(_),
            HistoryItem::Kicked(_),
            HistoryItem::ModeChanged(_),
            HistoryItem::TopicChanged(_),
            HistoryItem::NickChanged(_),
        ]
    ));
    assert!(!format!("{items:?}").contains("history-key-4286"));
    assert!(
        items[3]
            .as_mode_changed()
            .unwrap()
            .line
            .encode()
            .unwrap()
            .__has_secret_parameter()
    );
    let channel = session.state().channel("#papo").unwrap();
    assert_eq!(channel.topic(), Some("Current topic"));
    assert!(channel.member("bob").is_none());
    assert!(channel.member("alice-away").is_none());
    assert!(channel.member("alice").is_some());
}

#[test]
fn unsupported_batch_lines_neither_change_state_nor_trigger_replies() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default().ctcp(
            papo_core::CtcpReplies::default()
                .version("papo test build")
                .rate(RateLimit::unlimited()),
        ),
    );
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    while session.poll_event().is_some() {}
    let generation = session.state_generation();

    session.handle_line(now, b":znc.example BATCH +playback znc.in/playback #papo");
    for line in [
        "@batch=playback :alice!~a@remote JOIN #papo",
        "@batch=playback :papo!~p@localhost PART #papo :old",
        "@batch=playback :alice!~a@remote KICK #papo papo :old",
        "@batch=playback :alice!~a@remote MODE #papo +o alice",
        "@batch=playback :alice!~a@remote TOPIC #papo :old topic",
        "@batch=playback :alice!~a@remote NICK alice-away",
        "@batch=playback :alice!~a@remote PRIVMSG papo :\x01VERSION\x01",
    ] {
        session.handle_line(now, line.as_bytes());
        assert!(matches!(
            session.poll_event(),
            Some(Event::Line(line)) if line.batch_type() == Some("znc.in/playback")
        ));
    }
    session.handle_line(now, b":znc.example BATCH -playback");

    assert_eq!(session.state_generation(), generation);
    assert_eq!(session.poll_transmit(), None);
    assert_eq!(session.poll_event(), None);
}

#[test]
fn netjoin_for_our_nick_waits_for_requested_names() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().wanted_caps(["no-implicit-names"]));
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :no-implicit-names");
    session.handle_line(now, b"CAP * ACK :no-implicit-names");
    session.handle_line(now, b"BATCH +join netjoin irc.example irc.remote");
    session.handle_line(now, b"@batch=join :papo!papo@127.0.0.1 JOIN #papo");
    session.handle_line(now, b"BATCH -join");

    assert!(matches!(
        session.poll_event(),
        Some(Event::Joined {
            netjoin: Some(_),
            ..
        })
    ));
    assert!(!session.state().channel("#papo").unwrap().is_synced());
    session.handle_line(now, b":ergo.test 353 papo = #papo :@papo +alice");
    session.handle_line(now, b":ergo.test 366 papo #papo :End of NAMES list");
    assert!(matches!(
        session.poll_event(),
        Some(Event::ChannelSynced { channel, .. }) if channel == "#papo"
    ));
}
