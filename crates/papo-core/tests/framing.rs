//! Tests for stream framing and line decoding.

use std::time::Instant;

use papo_core::{Config, Decoding, Event, Session};

fn messages(session: &mut Session) -> Vec<String> {
    std::iter::from_fn(|| session.poll_event())
        .map(|event| match event {
            Event::Message(message) => message.to_line().unwrap(),
            other => panic!("expected message, got {other:?}"),
        })
        .collect()
}

#[test]
fn frames_split_reads_one_byte_at_a_time() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    for byte in b":ergo.test NOTICE * :*** Looking up your hostname...\r\n" {
        session.handle_bytes(now, &[*byte]);
    }
    assert_eq!(
        messages(&mut session),
        [":ergo.test NOTICE * :*** Looking up your hostname...\r\n"]
    );
}

#[test]
fn accepts_crlf_lf_empty_lines_and_many_lines_per_read() {
    let mut session = Session::new(Config::default());
    session.handle_bytes(
        Instant::now(),
        b"\r\n:ergo.test NOTICE * :First line\n\n:soju.im NOTICE * :Second line\r\n",
    );
    assert_eq!(
        messages(&mut session),
        [
            ":ergo.test NOTICE * :First line\r\n",
            ":soju.im NOTICE * :Second line\r\n"
        ]
    );
}

#[test]
fn falls_back_to_latin1_per_line() {
    let mut session = Session::new(Config::default());
    session.handle_bytes(Instant::now(), b":irc.example NOTICE nick :caf\xe9\r\n");
    assert_eq!(
        messages(&mut session),
        [":irc.example NOTICE nick caf\u{e9}\r\n"]
    );
}

#[test]
fn reports_strict_utf8_and_parse_failures_with_original_bytes() {
    let now = Instant::now();
    let mut session = Session::new(Config {
        decoding: Decoding::Utf8,
    });
    session.handle_line(now, b":irc.example NOTICE nick :caf\xe9");
    session.handle_line(now, b":ergo.test");
    assert_eq!(
        session.poll_event(),
        Some(Event::Malformed {
            line: b":irc.example NOTICE nick :caf\xe9".to_vec()
        })
    );
    assert_eq!(
        session.poll_event(),
        Some(Event::Malformed {
            line: b":ergo.test".to_vec()
        })
    );
}

#[test]
fn closes_only_over_the_literal_framing_limit() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_bytes(now, &vec![b'x'; 8191 + 512]);
    session.handle_bytes(now, b"\r");
    assert_eq!(session.poll_event(), None);
    session.handle_bytes(now, b"\n");
    assert!(matches!(
        session.poll_event(),
        Some(Event::Malformed { .. })
    ));

    session.handle_bytes(now, &vec![b'x'; 8191 + 512 + 1]);
    assert!(matches!(session.poll_event(), Some(Event::Close { .. })));
}

#[test]
fn reconnect_resets_partial_and_closed_framing_state() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_connected(now);
    session.handle_bytes(now, b":ergo.test NOTICE * :partial");
    session.handle_bytes(now, &vec![b'x'; 8191 + 512 + 1]);
    assert!(matches!(session.poll_event(), Some(Event::Close { .. })));

    session.handle_disconnected(now);
    session.handle_connected(now);
    session.handle_bytes(now, b"PING :after-reconnect\r\n");
    assert_eq!(
        session.poll_transmit(),
        Some(b"PONG :after-reconnect\r\n".to_vec())
    );
}
