//! Transcript coverage for channel list-mode requests.

use std::time::Instant;

use papo_core::{Config, Event, ListMode, RequestId, ResponseMatcher, Session};
use papo_proto::Line;

fn request(session: &mut Session, mode: ListMode) -> RequestId {
    session
        .request_with_matcher(
            Instant::now(),
            Line::new("MODE", ["#papo", mode_letter(mode)]),
            ResponseMatcher::ListMode {
                channel: "#papo".into(),
                mode,
            },
        )
        .unwrap()
}

fn mode_letter(mode: ListMode) -> &'static str {
    match mode {
        ListMode::Ban => "b",
        ListMode::Except => "e",
        ListMode::Invex => "I",
        ListMode::Quiet => "q",
        _ => unreachable!(),
    }
}

fn response(session: &mut Session, id: RequestId) -> papo_core::Response {
    std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::Response(response) if response.id == id => Some(response),
            _ => None,
        })
        .expect("list-mode response")
}

#[test]
fn each_list_mode_waits_for_its_end_numeric() {
    let cases = [
        (ListMode::Ban, 367, 368, "*!*@gateway.example"),
        (ListMode::Except, 348, 349, "*!*@staff.example"),
        (ListMode::Invex, 346, 347, "$a:invited"),
        (ListMode::Quiet, 728, 729, "*!*@noisy.example"),
    ];
    for (mode, row, end, mask) in cases {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        let id = request(&mut session, mode);
        let extra = if mode == ListMode::Quiet { " q" } else { "" };
        session.handle_line(
            now,
            format!(":solanum.example {row} papo #papo{extra} {mask} oper 1761048000").as_bytes(),
        );
        assert!(
            !std::iter::from_fn(|| session.poll_event())
                .any(|event| matches!(event, Event::Response(_)))
        );
        session.handle_line(
            now,
            format!(":solanum.example {end} papo #papo{extra} :End of mode list").as_bytes(),
        );
        assert_eq!(response(&mut session, id).lines.len(), 2);
    }
}

#[test]
fn an_empty_ban_list_completes_at_368() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    let id = request(&mut session, ListMode::Ban);
    session.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    assert_eq!(response(&mut session, id).lines.len(), 1);
}
