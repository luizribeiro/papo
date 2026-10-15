//! Transcript tests for outgoing text messages.

use std::time::Instant;

use papo_core::{Config, RateLimit, RequestError, Session, TextKind};
use papo_proto::Message;

fn multiline_session(now: Instant, value: &str, labels: bool) -> Session {
    let config = Config::default()
        .wanted_caps(["batch", "draft/multiline", "labeled-response"])
        .rate_limit(RateLimit::unlimited());
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
            format!(
                "@batch=b0-0;draft/multiline-concat PRIVMSG #papo {}\r\n",
                "a".repeat(420)
            ),
            format!("@batch=b0-0 PRIVMSG #papo {}\r\n", "a".repeat(180)),
            "BATCH -b0-0\r\n".into(),
        ]
    );
}

#[test]
fn max_lines_sends_several_bounded_batches() {
    let now = Instant::now();
    let mut session = multiline_session(now, "max-bytes=4096,max-lines=1", false);
    session
        .send_text(now, TextKind::Notice, "#papo", &"a".repeat(600), &[])
        .unwrap();
    let lines = drain(&mut session);
    assert_eq!(lines.len(), 6);
    assert_eq!(lines[0], "BATCH +b0-0 draft/multiline #papo\r\n");
    assert_eq!(lines[2], "BATCH -b0-0\r\n");
    assert_eq!(lines[3], "BATCH +b0-1 draft/multiline #papo\r\n");
    assert_eq!(lines[5], "BATCH -b0-1\r\n");
    assert!(!lines[1].contains("multiline-concat"));
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
    assert!(remaining[0].starts_with("@batch=b0-0;draft/multiline-concat"));
    assert!(remaining[1].starts_with("@batch=b0-0 PRIVMSG"));
    assert_eq!(remaining[2], "BATCH -b0-0\r\n");
    assert_eq!(remaining[3], "PONG :keepalive\r\n");
    assert_eq!(remaining[4], "PRIVMSG #elsewhere :queued later\r\n");
}
