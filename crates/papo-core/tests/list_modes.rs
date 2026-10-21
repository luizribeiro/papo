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
    session.handle_line(
        now,
        b":ergo.test 005 papo CHANMODES=beI,k,l,imnst :are supported",
    );
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

#[test]
fn live_mode_changes_update_only_a_completed_list() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(
        now,
        b":ergo.test 005 papo CHANMODES=beI,k,l,imnst :are supported",
    );
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let id = request(&mut session, ListMode::Ban);
    session.handle_line(now, b":ergo.test 367 papo #papo bob!*@* oper 1761048000");
    session.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    _ = response(&mut session, id);

    session.handle_line(
        now,
        b"@time=2026-10-21T12:00:00.000Z :alice!~alice@staff.example MODE #papo +bb carol bob",
    );
    let entries = session
        .state()
        .channel("#papo")
        .unwrap()
        .list(ListMode::Ban)
        .unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].mask, "carol");
    assert_eq!(entries[1].actor.as_ref().unwrap().to_string(), "alice");
    assert_eq!(
        entries[1].time,
        papo_proto::time::parse("2026-10-21T12:00:00.000Z")
    );

    session.handle_line(now, b":alice!~alice@staff.example MODE #papo -b bob");
    let entries = session
        .state()
        .channel("#papo")
        .unwrap()
        .list(ListMode::Ban)
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].mask, "carol");
}

#[test]
fn solanum_label_on_a_row_does_not_replace_the_unlabelled_end_numeric() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .wanted_caps(["labeled-response"])
            .rate_limit(papo_core::RateLimit::unlimited()),
    );
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :labeled-response");
    session.handle_line(now, b"CAP * ACK :labeled-response");
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    while session.poll_event().is_some() {}

    let id = request(&mut session, ListMode::Ban);
    assert_eq!(
        session.poll_transmit().unwrap(),
        b"@label=0 MODE #papo b\r\n"
    );
    session.handle_line(
        now,
        b"@label=0 :solanum.example 367 papo #papo *!*@gateway.example services 1761048000",
    );
    assert!(
        !std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::Response(_)))
    );
    session.handle_line(
        now,
        b":solanum.example 368 papo #papo :End of channel ban list",
    );
    assert_eq!(response(&mut session, id).mode_list.unwrap().len(), 1);
}

#[test]
fn bouncer_and_nonmember_fetches_are_returned_without_tracking() {
    let now = Instant::now();
    let mut bouncer = Session::new(Config::default());
    bouncer.handle_line(
        now,
        b":soju.example 005 papo BOUNCER_NETID=network-42 :are supported",
    );
    bouncer.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let id = request(&mut bouncer, ListMode::Ban);
    bouncer.handle_line(
        now,
        b":ergo.test 367 papo #papo *!*@gateway.example oper 1761048000",
    );
    bouncer.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    assert_eq!(response(&mut bouncer, id).mode_list.unwrap().len(), 1);
    assert_eq!(
        bouncer
            .state()
            .channel("#papo")
            .unwrap()
            .list(ListMode::Ban),
        None
    );

    let mut nonmember = Session::new(Config::default());
    let id = request(&mut nonmember, ListMode::Ban);
    nonmember.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    assert_eq!(response(&mut nonmember, id).mode_list, Some(Vec::new()));
    assert!(nonmember.state().channel("#papo").is_none());
}

#[test]
fn a_refusal_invalidates_a_previously_fetched_list() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let id = request(&mut session, ListMode::Ban);
    session.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    _ = response(&mut session, id);
    assert!(
        session
            .state()
            .channel("#papo")
            .unwrap()
            .list(ListMode::Ban)
            .is_some()
    );

    let id = request(&mut session, ListMode::Ban);
    session.handle_line(
        now,
        b":ergo.test 482 papo #papo :You're not a channel operator",
    );
    assert!(response(&mut session, id).error().is_some());
    assert_eq!(
        session
            .state()
            .channel("#papo")
            .unwrap()
            .list(ListMode::Ban),
        None
    );
}
