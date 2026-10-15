//! Transcript coverage for history requests and server replies.

use std::time::{Duration, Instant, UNIX_EPOCH};

use papo_core::{Config, Event, HistoryQuery, MsgRef, RateLimit, RequestError, Session};

fn history_session(isupport: &str) -> (Session, Instant) {
    let now = Instant::now();
    let mut config = Config::default().rate_limit(RateLimit::unlimited());
    config.wanted_caps = vec!["draft/chathistory".into(), "labeled-response".into()];
    let mut session = Session::new(config);
    session.handle_connected(now);
    session.handle_line(
        now,
        b"CAP * LS :draft/chathistory labeled-response server-time batch",
    );
    session.handle_line(
        now,
        b"CAP * ACK :draft/chathistory labeled-response server-time batch",
    );
    session.handle_line(
        now,
        format!(":ergo.test 005 papo {isupport} :are supported by this server").as_bytes(),
    );
    while session.poll_transmit().is_some() {}
    (session, now)
}

fn read_marker_session() -> (Session, Instant) {
    let now = Instant::now();
    let mut config = Config::default().rate_limit(RateLimit::unlimited());
    config.wanted_caps = vec!["draft/read-marker".into(), "labeled-response".into()];
    let mut session = Session::new(config);
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :draft/read-marker labeled-response");
    session.handle_line(now, b"CAP * ACK :draft/read-marker labeled-response");
    while session.poll_transmit().is_some() {}
    (session, now)
}

fn transmitted(session: &mut Session) -> String {
    String::from_utf8(session.poll_transmit().unwrap())
        .unwrap()
        .trim_end()
        .to_owned()
}

#[test]
fn builds_labelled_latest_and_before_requests() {
    let (mut session, now) = history_session("CHATHISTORY=1000 MSGREFTYPES=msgid,timestamp");
    session
        .chathistory(
            now,
            HistoryQuery::Latest {
                target: "#papo".into(),
                after: None,
                limit: 50,
            },
        )
        .unwrap();
    assert_eq!(
        transmitted(&mut session),
        "@label=0 CHATHISTORY LATEST #papo * 50"
    );
    session
        .chathistory(
            now,
            HistoryQuery::Before {
                target: "#papo".into(),
                before: MsgRef::Msgid("2Zl7JeYG6I8fG9q1lJ55BQ".into()),
                limit: 25,
            },
        )
        .unwrap();
    assert_eq!(
        transmitted(&mut session),
        "@label=1 CHATHISTORY BEFORE #papo msgid=2Zl7JeYG6I8fG9q1lJ55BQ 25"
    );
}

#[test]
fn uses_timestamps_when_they_are_the_only_reference_type() {
    let (mut session, now) = history_session("CHATHISTORY=1000 MSGREFTYPES=timestamp");
    session
        .chathistory(
            now,
            HistoryQuery::After {
                target: "#papo".into(),
                after: MsgRef::Timestamp(UNIX_EPOCH + Duration::new(1_760_529_342, 123_999_999)),
                limit: 50,
            },
        )
        .unwrap();
    assert_eq!(
        transmitted(&mut session),
        "@label=0 CHATHISTORY AFTER #papo timestamp=2025-10-15T11:55:42.123Z 50"
    );
    assert_eq!(
        session.chathistory(
            now,
            HistoryQuery::Before {
                target: "#papo".into(),
                before: MsgRef::Msgid("2Zl7JeYG6I8fG9q1lJ55BQ".into()),
                limit: 50,
            }
        ),
        Err(RequestError::InvalidMessageReference)
    );
    session.handle_line(
        now,
        b"@time=2025-10-15T11:55:43.000Z;msgid=2Zl7JeYG6I8fG9q1lJ55BQ :alice!~a@cloak PRIVMSG #papo :live",
    );
    session
        .chathistory(
            now,
            HistoryQuery::Before {
                target: "#papo".into(),
                before: MsgRef::Msgid("2Zl7JeYG6I8fG9q1lJ55BQ".into()),
                limit: 50,
            },
        )
        .unwrap();
    assert_eq!(
        transmitted(&mut session),
        "@label=1 CHATHISTORY BEFORE #papo timestamp=2025-10-15T11:55:43.000Z 50"
    );
}

#[test]
fn clamps_server_and_local_limits() {
    let (mut limited, now) = history_session("CHATHISTORY=100 MSGREFTYPES=msgid,timestamp");
    limited
        .chathistory(
            now,
            HistoryQuery::Latest {
                target: "#papo".into(),
                after: None,
                limit: 500,
            },
        )
        .unwrap();
    assert!(transmitted(&mut limited).ends_with(" #papo * 100"));

    let (mut unlimited, now) = history_session("CHATHISTORY=0 MSGREFTYPES=msgid,timestamp");
    unlimited
        .chathistory(
            now,
            HistoryQuery::Latest {
                target: "#papo".into(),
                after: None,
                limit: 1_001,
            },
        )
        .unwrap();
    assert!(transmitted(&mut unlimited).ends_with(" #papo * 1000"));
}

#[test]
fn rejects_history_without_its_capability() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    assert_eq!(
        session.chathistory(
            now,
            HistoryQuery::Latest {
                target: "#papo".into(),
                after: None,
                limit: 50,
            }
        ),
        Err(RequestError::Unsupported("chathistory"))
    );
}

#[test]
fn assembles_deduplicated_context_aware_pages() {
    let (mut session, now) = history_session("CHATHISTORY=1000 MSGREFTYPES=msgid,timestamp");
    session
        .chathistory(
            now,
            HistoryQuery::Latest {
                target: "#papo".into(),
                after: None,
                limit: 50,
            },
        )
        .unwrap();
    let _ = session.poll_transmit();
    for line in [
        "@label=0 :ergo.test BATCH +reply labeled-response",
        "@batch=reply;draft/chathistory-end :ergo.test BATCH +history chathistory #papo",
        "@batch=history;msgid=RKi9Cw :alice!~a@localhost PRIVMSG #papo :earlier",
        "@batch=history;msgid=RKi9Cw :alice!~a@localhost PRIVMSG #papo :duplicate",
        "@batch=history;msgid=T1pQ6g;draft/chathistory-context :alice!~a@localhost TAGMSG #papo",
        "@batch=reply :ergo.test BATCH -history",
        ":ergo.test BATCH -reply",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    assert!(
        matches!(session.poll_event(), Some(Event::History { messages, end: true, .. }) if messages.len() == 2)
    );
    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("expected history response")
    };
    let page = response.history.unwrap();
    assert_eq!(page.target, "#papo");
    assert_eq!(page.messages[0].as_ref().param(1), Some("earlier"));
    assert!(!page.messages[0].context);
    assert!(page.messages[1].context);
}

#[test]
fn correlates_chathistory_failures() {
    let (mut session, now) = history_session("CHATHISTORY=1000 MSGREFTYPES=msgid,timestamp");
    session
        .chathistory(
            now,
            HistoryQuery::Latest {
                target: "#missing".into(),
                after: None,
                limit: 50,
            },
        )
        .unwrap();
    session.handle_line(
        now,
        b"@label=0 :ergo.test FAIL CHATHISTORY INVALID_TARGET #missing :Messages could not be retrieved",
    );
    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("expected failed response")
    };
    assert!(response.error().is_some_and(|error| matches!(error, papo_core::ServerError::Standard { code, .. } if code == "INVALID_TARGET")));
}

#[test]
fn reports_targets_with_parsed_server_times() {
    let (mut session, now) = history_session("CHATHISTORY=1000 MSGREFTYPES=msgid,timestamp");
    session
        .chathistory(
            now,
            HistoryQuery::Targets {
                from: UNIX_EPOCH + Duration::from_secs(1_760_529_301),
                to: UNIX_EPOCH + Duration::from_secs(1_760_529_401),
                limit: 20,
            },
        )
        .unwrap();
    assert!(transmitted(&mut session).contains("CHATHISTORY TARGETS "));
    for line in [
        "@label=0 :ergo.test BATCH +reply labeled-response",
        "@batch=reply :ergo.test BATCH +targets draft/chathistory-targets",
        "@batch=targets :ergo.test CHATHISTORY TARGETS #papo 2025-10-15T11:55:42.123Z",
        "@batch=targets :ergo.test CHATHISTORY TARGETS Alice 2025-10-15T11:55:43.000Z",
        "@batch=reply :ergo.test BATCH -targets",
        ":ergo.test BATCH -reply",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    let Some(Event::HistoryTargets(targets)) = session.poll_event() else {
        panic!("expected history targets")
    };
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].0, "#papo");
    assert_eq!(
        targets[0].1,
        UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000)
    );
}

#[test]
fn sends_read_marker_set_and_query_commands() {
    let (mut session, now) = read_marker_session();
    session
        .mark_read(
            now,
            "#papo",
            UNIX_EPOCH + Duration::new(1_760_529_342, 123_999_999),
        )
        .unwrap();
    assert_eq!(
        transmitted(&mut session),
        "@label=0 MARKREAD #papo timestamp=2025-10-15T11:55:42.123Z"
    );
    session.read_marker("#papo").unwrap();
    assert_eq!(transmitted(&mut session), "MARKREAD #papo");
}

#[test]
fn rejects_read_markers_without_the_capability() {
    let mut session = Session::new(Config::default());
    assert_eq!(
        session.read_marker("#papo"),
        Err(RequestError::Unsupported("read-marker"))
    );
}

#[test]
fn tracks_read_markers_monotonically_through_join_bursts() {
    let (mut session, now) = read_marker_session();
    session.handle_line(
        now,
        b"@label=remote :ergo.test MARKREAD #papo timestamp=2025-10-15T11:55:42.123Z",
    );
    let Some(Event::ReadMarker(marker)) = session.poll_event() else {
        panic!("expected read marker")
    };
    assert_eq!(marker.target, "#papo");
    assert_eq!(
        marker.time,
        Some(UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000))
    );
    for line in [
        ":ergo.test MARKREAD #papo timestamp=2025-10-15T11:55:41.999Z",
        ":ergo.test MARKREAD #papo timestamp=*",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    assert_eq!(session.poll_event(), None);
    assert_eq!(
        session
            .state()
            .read_marker("#PAPO")
            .and_then(|marker| marker.time),
        marker.time
    );

    session.handle_line(now, b":papo!~papo@localhost JOIN #joined * :Papo Client");
    assert!(matches!(session.poll_event(), Some(Event::Joined { .. })));
    session.handle_line(now, b":ergo.test MARKREAD #joined *");
    assert!(
        matches!(session.poll_event(), Some(Event::ReadMarker(marker)) if marker.target == "#joined" && marker.time.is_none())
    );
    session.handle_line(now, b":ergo.test 366 papo #joined :End of /NAMES list");
    assert!(
        matches!(session.poll_event(), Some(Event::ChannelSynced { channel }) if channel == "#joined")
    );
}

#[test]
fn markread_failures_remain_standard_replies() {
    let mut session = Session::new(Config::default());
    session.handle_line(
        Instant::now(),
        b":ergo.test FAIL MARKREAD INVALID_PARAMS #papo :Invalid parameters",
    );
    assert!(
        matches!(session.poll_event(), Some(Event::StandardReply(reply)) if reply.command == "MARKREAD" && reply.code == "INVALID_PARAMS")
    );
}

#[test]
fn remembers_live_messages_but_not_history_pages() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    session.handle_line(
        now,
        b"@time=2025-10-15T11:55:42.123Z;msgid=RKi9Cw :alice!~a@cloak PRIVMSG #papo :live",
    );
    let seen = session.state().last_seen("#PAPO").unwrap();
    assert_eq!(seen.msgid, "RKi9Cw");
    assert_eq!(
        seen.time,
        UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000)
    );

    for line in [
        ":ergo.test BATCH +history chathistory #papo",
        "@batch=history;time=2025-10-15T11:55:43.000Z;msgid=T1pQ6g :bob!~b@cloak PRIVMSG #papo :replay",
        ":ergo.test BATCH -history",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    assert!(matches!(session.poll_event(), Some(Event::Message(_))));
    assert!(matches!(session.poll_event(), Some(Event::History { .. })));
    assert_eq!(session.state().last_seen("#papo").unwrap().msgid, "RKi9Cw");

    session.handle_disconnected(now);
    session.handle_connected(now);
    assert_eq!(session.state().last_seen("#papo").unwrap().msgid, "RKi9Cw");
    session.handle_line(
        now,
        b"@time=2025-10-15T11:55:44.000Z;msgid=8sKY2A :alice!~a@cloak NOTICE papo :private",
    );
    assert_eq!(session.state().last_seen("Alice").unwrap().msgid, "8sKY2A");
}
