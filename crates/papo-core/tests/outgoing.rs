//! Transcript tests for outgoing text messages.

use std::time::{Duration, Instant};

use papo_core::{Config, Event, RateLimit, RequestError, ServerError, Session, TextKind};
use papo_proto::Message;

fn multiline_session(now: Instant, value: &str, labels: bool) -> Session {
    multiline_session_with_rate_limit(now, value, labels, RateLimit::unlimited())
}

fn multiline_session_with_rate_limit(
    now: Instant,
    value: &str,
    labels: bool,
    rate_limit: RateLimit,
) -> Session {
    let config = Config::default()
        .wanted_caps(["batch", "draft/multiline", "labeled-response"])
        .rate_limit(rate_limit);
    let mut session = Session::new(config);
    session.handle_connected(now);
    let labels = if labels { " labeled-response" } else { "" };
    session.handle_line(
        now,
        format!("CAP * LS :batch draft/multiline={value}{labels}").as_bytes(),
    );
    session.handle_line(
        now,
        format!("CAP * ACK :batch draft/multiline{labels}").as_bytes(),
    );
    while session.poll_transmit().is_some() {}
    session
}

fn drain(session: &mut Session) -> Vec<String> {
    std::iter::from_fn(|| session.poll_transmit())
        .map(|line| String::from_utf8(line).unwrap())
        .collect()
}

#[test]
fn splits_a_six_hundred_byte_privmsg_at_the_literal_budget() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    let text = "a".repeat(600);
    assert_eq!(
        session.send_text(now, TextKind::Privmsg, "#papo", &text, &[]),
        Ok(None)
    );
    assert_eq!(
        session.poll_transmit(),
        Some(format!("PRIVMSG #papo {}\r\n", "a".repeat(420)).into_bytes())
    );
    assert_eq!(
        session.poll_transmit(),
        Some(format!("PRIVMSG #papo {}\r\n", "a".repeat(180)).into_bytes())
    );
    assert_eq!(session.poll_transmit(), None);
}

#[test]
fn repeats_client_tags_except_context_on_fallback_lines() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    let tags = [("+reply", "RKi9Cw"), ("+example.com/device", "web")];
    session
        .send_text(now, TextKind::Notice, "#papo", &"a".repeat(600), &tags)
        .unwrap();
    let first = String::from_utf8(session.poll_transmit().unwrap()).unwrap();
    let second = String::from_utf8(session.poll_transmit().unwrap()).unwrap();
    assert!(first.starts_with("@+reply=RKi9Cw;+example.com/device=web NOTICE"));
    assert!(second.starts_with("@+example.com/device=web NOTICE"));
    assert!(!second.contains("+reply"));
}

#[test]
fn rejects_empty_and_newline_only_text_without_queueing() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    for text in ["", "\n\n"] {
        assert_eq!(
            session.send_text(now, TextKind::Privmsg, "#papo", text, &[]),
            Err(RequestError::EmptyText)
        );
    }
    assert_eq!(session.poll_transmit(), None);
}

#[test]
fn sends_six_hundred_bytes_as_one_labelled_multiline_batch() {
    let now = Instant::now();
    let mut session = multiline_session(now, "max-bytes=4096,max-lines=100", true);
    let tags = [("+reply", "RKi9Cw")];
    session
        .send_text(now, TextKind::Privmsg, "#papo", &"a".repeat(600), &tags)
        .unwrap();
    assert_eq!(
        drain(&mut session),
        [
            "@+reply=RKi9Cw;label=0 BATCH +b0-0 draft/multiline #papo\r\n".into(),
            format!("@batch=b0-0 PRIVMSG #papo {}\r\n", "a".repeat(420)),
            format!(
                "@batch=b0-0;draft/multiline-concat PRIVMSG #papo {}\r\n",
                "a".repeat(180)
            ),
            "BATCH -b0-0\r\n".into(),
        ]
    );
}

#[test]
fn max_bytes_counts_text_and_joining_newlines() {
    let now = Instant::now();
    let mut exact = multiline_session(now, "max-bytes=40,max-lines=3", false);
    exact
        .send_text(
            now,
            TextKind::Notice,
            "#papo",
            "1234567890123456789\nabcdefghijklmnopqrst",
            &[],
        )
        .unwrap();
    assert_eq!(
        drain(&mut exact),
        [
            "BATCH +b0-0 draft/multiline #papo\r\n",
            "@batch=b0-0 NOTICE #papo 1234567890123456789\r\n",
            "@batch=b0-0 NOTICE #papo abcdefghijklmnopqrst\r\n",
            "BATCH -b0-0\r\n",
        ]
    );

    let mut over = multiline_session(now, "max-bytes=40,max-lines=3", false);
    over.send_text(
        now,
        TextKind::Notice,
        "#papo",
        "1234567890123456789\nabcdefghijklmnopqrstu",
        &[],
    )
    .unwrap();
    let lines = drain(&mut over);
    assert_eq!(lines.len(), 6);
    assert_eq!(lines[2], "BATCH -b0-0\r\n");
    assert_eq!(lines[3], "BATCH +b0-1 draft/multiline #papo\r\n");
}

#[test]
fn concat_omits_the_joining_newline_from_max_bytes() {
    let now = Instant::now();
    let mut concat = multiline_session(now, "max-bytes=800,max-lines=3", false);
    concat
        .send_text(now, TextKind::Privmsg, "#papo", &"a".repeat(800), &[])
        .unwrap();
    let lines = drain(&mut concat);
    assert_eq!(lines.len(), 4);
    assert!(!lines[1].contains("draft/multiline-concat"));
    assert!(lines[2].contains("draft/multiline-concat"));

    let mut joined = multiline_session(now, "max-bytes=800,max-lines=3", false);
    let text = format!("{}\n{}", "a".repeat(420), "b".repeat(380));
    joined
        .send_text(now, TextKind::Privmsg, "#papo", &text, &[])
        .unwrap();
    assert_eq!(
        drain(&mut joined)
            .iter()
            .filter(|line| line.starts_with("BATCH +"))
            .count(),
        2
    );
}

#[test]
fn max_lines_accepts_three_lines_and_splits_the_fourth() {
    let now = Instant::now();
    let mut exact = multiline_session(now, "max-bytes=40,max-lines=3", false);
    exact
        .send_text(now, TextKind::Notice, "#papo", "one\ntwo\nthree", &[])
        .unwrap();
    let lines = drain(&mut exact);
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[4], "BATCH -b0-0\r\n");

    let mut over = multiline_session(now, "max-bytes=40,max-lines=3", false);
    let tags = [("+reply", "RKi9Cw"), ("+example.com/device", "web")];
    over.send_text(
        now,
        TextKind::Notice,
        "#papo",
        "one\ntwo\nthree\nfour",
        &tags,
    )
    .unwrap();
    let lines = drain(&mut over);
    assert_eq!(lines.len(), 8);
    assert!(lines[0].starts_with("@+reply=RKi9Cw;+example.com/device=web BATCH"));
    assert_eq!(lines[4], "BATCH -b0-0\r\n");
    assert!(lines[5].starts_with("@+example.com/device=web BATCH +b0-1"));
    assert!(!lines[5].contains("+reply"));
    assert_eq!(lines[7], "BATCH -b0-1\r\n");
}

#[test]
fn max_bytes_without_max_lines_accepts_all_lines() {
    let now = Instant::now();
    let mut session = multiline_session(now, "max-bytes=40", false);
    session
        .send_text(now, TextKind::Notice, "#papo", "one\ntwo\nthree\nfour", &[])
        .unwrap();
    let lines = drain(&mut session);
    assert_eq!(lines.len(), 6);
    assert_eq!(lines[0], "BATCH +b0-0 draft/multiline #papo\r\n");
    assert_eq!(lines[5], "BATCH -b0-0\r\n");
}

#[test]
fn invalid_multiline_limits_fall_back_to_plain_lines() {
    let now = Instant::now();
    for value in [
        "max-bytes=0,max-lines=3",
        "max-bytes=many,max-lines=3",
        "max-lines=3",
        "max-bytes=40,max-lines=0",
    ] {
        let mut session = multiline_session(now, value, false);
        session
            .send_text(
                now,
                TextKind::Privmsg,
                "#papo",
                "hello everyone\nwelcome back",
                &[],
            )
            .unwrap();
        assert_eq!(
            drain(&mut session),
            [
                "PRIVMSG #papo :hello everyone\r\n",
                "PRIVMSG #papo :welcome back\r\n",
            ],
            "capability value: {value}"
        );
    }

    let mut labelled = multiline_session(now, "max-bytes=many,max-lines=3", true);
    labelled
        .send_text(
            now,
            TextKind::Privmsg,
            "#papo",
            "hello everyone\nwelcome back",
            &[],
        )
        .unwrap();
    assert_eq!(
        drain(&mut labelled),
        [
            "@label=0 PRIVMSG #papo :hello everyone\r\n",
            "PRIVMSG #papo :welcome back\r\n",
        ]
    );
}

#[test]
fn later_batches_mark_only_internal_concat_segments() {
    let now = Instant::now();
    let mut session = multiline_session(now, "max-bytes=1680,max-lines=4", false);
    session
        .send_text(now, TextKind::Privmsg, "#papo", &"a".repeat(3361), &[])
        .unwrap();
    let lines = drain(&mut session);
    assert_eq!(lines.len(), 15);
    assert!(!lines[7].contains("draft/multiline-concat"));
    for line in &lines[8..11] {
        assert!(line.contains("draft/multiline-concat"));
    }
    assert!(!lines[13].contains("draft/multiline-concat"));
}

#[test]
fn every_batch_wire_line_consumes_one_rate_limit_token() {
    let now = Instant::now();
    let mut session = multiline_session_with_rate_limit(
        now,
        "max-bytes=40,max-lines=3",
        false,
        RateLimit::new(4, Duration::from_millis(500)).unwrap(),
    );
    session
        .send_text(now, TextKind::Privmsg, "#papo", "one\ntwo\nthree", &[])
        .unwrap();
    assert_eq!(session.queued_len(), 5);
    for (expected, remaining) in [
        ("BATCH +b0-0 draft/multiline #papo\r\n", 4),
        ("@batch=b0-0 PRIVMSG #papo one\r\n", 3),
        ("@batch=b0-0 PRIVMSG #papo two\r\n", 2),
        ("@batch=b0-0 PRIVMSG #papo three\r\n", 1),
    ] {
        assert_eq!(session.poll_transmit(), Some(expected.as_bytes().to_vec()));
        assert_eq!(session.queued_len(), remaining);
    }
    assert_eq!(session.poll_transmit(), None);
    assert_eq!(session.queued_len(), 1);

    session.handle_timeout(now + Duration::from_millis(500));
    assert_eq!(session.poll_transmit(), Some(b"BATCH -b0-0\r\n".to_vec()));
    assert_eq!(session.queued_len(), 0);
}

#[test]
fn interleaved_traffic_waits_for_the_batch_close() {
    let now = Instant::now();
    let mut session = multiline_session(now, "max-bytes=4096,max-lines=100", false);
    session
        .send_text(now, TextKind::Privmsg, "#papo", &"a".repeat(600), &[])
        .unwrap();
    assert_eq!(
        String::from_utf8(session.poll_transmit().unwrap()).unwrap(),
        "BATCH +b0-0 draft/multiline #papo\r\n"
    );
    session
        .send(now, Message::privmsg("#elsewhere", "queued later"))
        .unwrap();
    session.handle_line(now, b"PING :keepalive");
    let remaining = drain(&mut session);
    assert!(remaining[0].starts_with("@batch=b0-0 PRIVMSG"));
    assert!(remaining[1].starts_with("@batch=b0-0;draft/multiline-concat"));
    assert_eq!(remaining[2], "BATCH -b0-0\r\n");
    assert_eq!(remaining[3], "PONG :keepalive\r\n");
    assert_eq!(remaining[4], "PRIVMSG #elsewhere :queued later\r\n");
}

#[test]
fn multiline_failure_is_a_response_error_without_retry() {
    let now = Instant::now();
    let mut session = multiline_session(now, "max-bytes=4096,max-lines=100", true);
    let id = session
        .send_text(now, TextKind::Privmsg, "#papo", "hello everyone", &[])
        .unwrap()
        .unwrap();
    drain(&mut session);
    session.handle_line(
        now,
        b"@label=0 :ergo.test FAIL BATCH MULTILINE_MAX_LINES #papo :Too many lines",
    );
    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("expected labelled failure response")
    };
    assert_eq!(response.id, id);
    assert!(matches!(
        response.error(),
        Some(ServerError::Standard { command, code, .. })
            if command == "BATCH" && code == "MULTILINE_MAX_LINES"
    ));
    assert_eq!(session.poll_transmit(), None);
}

#[test]
fn text_response_retains_labelled_echo_identity() {
    let now = Instant::now();
    let mut session =
        Session::new(Config::default().wanted_caps(["echo-message", "labeled-response"]));
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :echo-message labeled-response");
    session.handle_line(now, b"CAP * ACK :echo-message labeled-response");
    while session.poll_transmit().is_some() {}
    let id = session
        .send_text(now, TextKind::Privmsg, "#papo", "hello everyone", &[])
        .unwrap()
        .unwrap();
    session.handle_line(
        now,
        b"@label=0;msgid=RfcBTzyxY120004;time=2026-02-03T04:05:06.789Z :papo!p@localhost PRIVMSG #papo :hello everyone",
    );
    let response = std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::Response(response) if response.id == id => Some(response),
            _ => None,
        })
        .unwrap();
    assert_eq!(response.sent().unwrap().msgid, "RfcBTzyxY120004");
}
