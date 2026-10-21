//! Transcript coverage for channel list-mode requests.

use std::time::{Duration, Instant, UNIX_EPOCH};

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
        session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
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
        let response = response(&mut session, id);
        let entries = response.mode_list.unwrap();
        assert_eq!(response.lines.len(), 2);
        assert_eq!(entries[0].mask, mask);
        assert_eq!(entries[0].actor.as_ref().unwrap().to_string(), "oper");
        assert_eq!(
            entries[0].time,
            UNIX_EPOCH.checked_add(Duration::from_hours(489_180))
        );
        assert_eq!(
            session.state().channel("#papo").unwrap().list(mode),
            Some(entries.as_slice())
        );
    }
}

#[test]
fn an_empty_ban_list_completes_at_368() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let id = request(&mut session, ListMode::Ban);
    session.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    let response = response(&mut session, id);
    assert_eq!(response.lines.len(), 1);
    assert_eq!(response.mode_list, Some(Vec::new()));
    assert_eq!(
        session
            .state()
            .channel("#papo")
            .unwrap()
            .list(ListMode::Ban),
        Some([].as_slice())
    );
}

#[test]
fn one_thousand_and_one_rows_fail_without_retaining_the_last_row() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let id = request(&mut session, ListMode::Ban);
    let rows: Vec<_> = (0..1001)
        .map(|index| format!(":ergo.test 367 papo #papo *!*@host{index}.example oper 1761048000"))
        .collect();
    for row in &rows[..1000] {
        session.handle_line(now, row.as_bytes());
    }
    assert!(
        !std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::Response(_)))
    );

    session.handle_line(now, rows[1000].as_bytes());
    assert!(matches!(
        session.poll_event(),
        Some(Event::StateLimitReached {
            limit: papo_core::StateLimit::ListEntries
        })
    ));
    let response = response(&mut session, id);
    assert_eq!(response.lines.len(), 1000);
    assert_eq!(
        response.state_limit,
        Some(papo_core::StateLimit::ListEntries)
    );
    assert_eq!(
        session
            .state()
            .channel("#papo")
            .unwrap()
            .list(ListMode::Ban),
        None
    );
}
