//! Transcript coverage for soju search batch routing.
#![cfg(feature = "unstable")]

use std::time::Instant;

use papo_core::{Config, Event, Session};
use papo_proto::Line;

fn search_session(now: Instant, max_lines: usize) -> Session {
    let mut session = Session::new(Config::default().max_batch_lines(max_lines));
    session.handle_line(now, b"CAP * LS :batch labeled-response soju.im/search");
    session.handle_line(now, b"CAP * ACK :batch labeled-response soju.im/search");
    while session.poll_transmit().is_some() {}
    session
}

#[test]
fn routes_soju_search_only_to_its_opening_request() {
    let now = Instant::now();
    let mut session = search_session(now, 10);
    let id = session
        .request_silent(
            now,
            Line::new("SEARCH").with_params(["in=#papo", "text=release"]),
        )
        .unwrap();
    assert_eq!(
        String::from_utf8(session.poll_transmit().unwrap().to_vec()).unwrap(),
        "@label=0 SEARCH in=#papo text=release\r\n"
    );
    session.handle_line(
        now,
        b"@label=0 :soju.example BATCH +search-42 soju.im/search",
    );
    session.handle_line(
        now,
        b"@batch=search-42 :alice!~a@client.example PRIVMSG #papo :release is ready",
    );
    session.handle_line(now, b":soju.example BATCH -search-42");

    assert!(
        matches!(session.poll_event(), Some(Event::Response(response))
        if response.id == id && response.lines.len() == 1)
    );
    assert_eq!(session.poll_event(), None);
}
