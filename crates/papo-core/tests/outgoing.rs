//! Transcript tests for outgoing text messages.

use std::time::Instant;

use papo_core::{Config, RateLimit, RequestError, Session, TextKind};

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
