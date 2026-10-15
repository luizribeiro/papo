//! Transcript coverage for history requests and server replies.

use std::time::{Duration, Instant, UNIX_EPOCH};

use papo_core::{Config, HistoryQuery, MsgRef, RateLimit, RequestError, Session};

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
