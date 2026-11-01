//! Transcript coverage for channel list-mode requests.

use std::time::{Duration, Instant, UNIX_EPOCH};

use papo_core::{
    Config, Event, ListMode, RequestError, RequestId, ResponseMatcher, Session, StateLimit,
};
use papo_proto::Line;

fn request(session: &mut Session, now: Instant, mode: ListMode) -> RequestId {
    session
        .request_with_matcher(
            now,
            Line::new("MODE").with_params(["#papo", mode_letter(mode)]),
            ResponseMatcher::__list_mode("#papo".into(), mode),
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
        let id = request(&mut session, now, mode);
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
        assert_eq!(entries[0].actor().unwrap().to_string(), "oper");
        assert_eq!(
            entries[0].set_at,
            UNIX_EPOCH.checked_add(Duration::from_hours(489_180))
        );
        assert_eq!(
            session.state().channel("#papo").unwrap().list(mode),
            Some(entries.as_slice())
        );
    }
}

#[test]
fn list_requests_ignore_other_dialects_targets_and_quiet_markers() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    let id = request(&mut session, now, ListMode::Ban);
    for line in [
        ":ergo.test 348 papo #papo *!*@staff.example oper 1761048000",
        ":ergo.test 367 papo #other *!*@gateway.example oper 1761048000",
        ":ergo.test 482 papo #other :You're not a channel operator",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    session.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    let ban_response = response(&mut session, id);
    assert_eq!(ban_response.lines.len(), 1);
    assert_eq!(ban_response.lines[0].as_ref().command().as_str(), "368");

    let id = request(&mut session, now, ListMode::Quiet);
    session.handle_line(
        now,
        b":solanum.example 728 papo #papo b *!*@noisy.example oper 1761048000",
    );
    session.handle_line(
        now,
        b":solanum.example 729 papo #papo b :End of channel ban list",
    );
    session.handle_line(
        now,
        b":solanum.example 729 papo #papo q :End of channel quiet list",
    );
    let response = response(&mut session, id);
    assert_eq!(response.lines.len(), 1);
    assert_eq!(response.lines[0].as_ref().param(2), Some("q"));
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
    let id = request(&mut session, now, ListMode::Ban);
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
fn list_mode_limit_counts_rows_without_the_completion_numeric() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().max_list_entries_per_mode(1));
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let id = request(&mut session, now, ListMode::Ban);
    _ = session.poll_transmit();
    session.handle_line(
        now,
        b":ergo.test 367 papo #papo *!*@gateway.example oper 1761048000",
    );
    session.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    let response = response(&mut session, id);
    assert_eq!(response.lines.len(), 2);
    assert_eq!(
        session
            .state()
            .channel("#papo")
            .unwrap()
            .list(ListMode::Ban)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn list_mode_snapshot_refuses_the_third_row_at_a_limit_of_two() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().max_list_entries_per_mode(2));
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let id = request(&mut session, now, ListMode::Ban);
    _ = session.poll_transmit();
    for host in ["gateway.example", "crawler.example"] {
        session.handle_line(
            now,
            format!(":ergo.test 367 papo #papo *!*@{host} oper 1761048000").as_bytes(),
        );
    }
    assert!(
        !std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::Response(_)))
    );

    session.handle_line(
        now,
        b":ergo.test 367 papo #papo *!*@scanner.example oper 1761048000",
    );
    assert!(matches!(
        session.poll_event(),
        Some(Event::StateLimitReached {
            limit: StateLimit::ListEntries,
            ..
        })
    ));
    let response = response(&mut session, id);
    assert_eq!(response.state_limit, Some(StateLimit::ListEntries));
    assert_eq!(response.lines.len(), 2);
}

#[test]
fn one_thousand_and_one_rows_fail_without_retaining_the_last_row() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let id = request(&mut session, now, ListMode::Ban);
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
            limit: papo_core::StateLimit::ListEntries,
            ..
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
fn list_mode_changes_refuse_growth_when_the_cached_list_is_full() {
    let now = Instant::now();
    let config = Config::default().max_list_entries_per_mode(1);
    let mut below = Session::new(config.clone());
    below.handle_line(
        now,
        b":ergo.test 005 papo CHANMODES=beI,k,l,imnst :are supported",
    );
    below.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let empty = request(&mut below, now, ListMode::Ban);
    _ = below.poll_transmit();
    below.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    _ = response(&mut below, empty);
    below
        .request_with_matcher(
            now,
            Line::new("MODE").with_params(["#papo", "+b", "*!*@gateway.example"]),
            ResponseMatcher::Mode("#papo".into()),
        )
        .unwrap();
    assert_eq!(
        String::from_utf8(below.poll_transmit().unwrap().to_vec()).unwrap(),
        "MODE #papo +b *!*@gateway.example\r\n"
    );

    let mut full = Session::new(config);
    full.handle_line(
        now,
        b":ergo.test 005 papo CHANMODES=beI,k,l,imnst :are supported",
    );
    full.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let list = request(&mut full, now, ListMode::Ban);
    _ = full.poll_transmit();
    full.handle_line(
        now,
        b":ergo.test 367 papo #papo *!*@gateway.example oper 1761048000",
    );
    full.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    _ = response(&mut full, list);
    full.request_with_matcher(
        now,
        Line::new("MODE").with_params(["#papo", "+b", "*!*@gateway.example"]),
        ResponseMatcher::Mode("#papo".into()),
    )
    .unwrap();
    _ = full.poll_transmit();
    _ = full.poll_transmit();
    assert_eq!(
        full.request_with_matcher(
            now,
            Line::new("MODE").with_params(["#papo", "+b", "*!*@crawler.example"]),
            ResponseMatcher::Mode("#papo".into()),
        ),
        Err(RequestError::StateLimit(StateLimit::ListEntries))
    );
    assert_eq!(full.poll_transmit(), None);

    for mask in ["*!*@crawler.example", "*!*@scanner.example"] {
        full.handle_line(
            now,
            format!(":oper!admin@staff.example MODE #papo +b {mask}").as_bytes(),
        );
    }
    let warnings = std::iter::from_fn(|| full.poll_event())
        .filter(|event| {
            matches!(
                event,
                Event::StateLimitReached {
                    limit: StateLimit::ListEntries,
                    ..
                }
            )
        })
        .count();
    assert_eq!(warnings, 1);
    assert_eq!(
        full.state()
            .channel("#papo")
            .unwrap()
            .list(ListMode::Ban)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn every_cached_list_mode_refuses_growth_at_its_limit() {
    let cases = [
        (ListMode::Ban, 367, 368, "", "*!*@gateway.example"),
        (ListMode::Except, 348, 349, "", "*!*@staff.example"),
        (ListMode::Invex, 346, 347, "", "$a:invited"),
        (ListMode::Quiet, 728, 729, " q", "*!*@noisy.example"),
    ];
    for (mode, row, end, extra, mask) in cases {
        let now = Instant::now();
        let mut session = Session::new(Config::default().max_list_entries_per_mode(1));
        session.handle_line(
            now,
            b":solanum.example 005 papo CHANMODES=eIbq,k,flj,CFLPQcgimnprstz :are supported",
        );
        session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
        let id = request(&mut session, now, mode);
        _ = session.poll_transmit();
        session.handle_line(
            now,
            format!(":solanum.example {row} papo #papo{extra} {mask} oper 1761048000").as_bytes(),
        );
        session.handle_line(
            now,
            format!(":solanum.example {end} papo #papo{extra} :End of mode list").as_bytes(),
        );
        _ = response(&mut session, id);

        let letter = mode_letter(mode);
        assert_eq!(
            session.request_with_matcher(
                now,
                Line::new("MODE").with_params(["#papo", &format!("+{letter}"), "*!*@new.example"]),
                ResponseMatcher::Mode("#papo".into()),
            ),
            Err(RequestError::StateLimit(StateLimit::ListEntries))
        );
        assert_eq!(session.poll_transmit(), None);
    }
}

#[test]
fn a_full_cached_list_allows_an_atomic_replacement() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().max_list_entries_per_mode(1));
    session.handle_line(
        now,
        b":ergo.test 005 papo CHANMODES=beI,k,l,imnst :are supported",
    );
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let id = request(&mut session, now, ListMode::Ban);
    _ = session.poll_transmit();
    session.handle_line(
        now,
        b":ergo.test 367 papo #papo *!*@gateway.example oper 1761048000",
    );
    session.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    _ = response(&mut session, id);

    session
        .request_with_matcher(
            now,
            Line::new("MODE").with_params([
                "#papo",
                "-b+b",
                "*!*@gateway.example",
                "*!*@crawler.example",
            ]),
            ResponseMatcher::Mode("#papo".into()),
        )
        .unwrap();
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"MODE #papo -b+b *!*@gateway.example *!*@crawler.example\r\n"
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
    let id = request(&mut session, now, ListMode::Ban);
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
    assert!(matches!(
        entries[1].actor().unwrap(),
        papo_core::Source::User {
            nick,
            username: Some(username),
            host: Some(host),
            ..
        } if nick == "alice" && username == "~alice" && host == "staff.example"
    ));
    assert_eq!(
        entries[1].set_at,
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
fn solanum_live_changes_update_except_invex_and_quiet_lists() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(
        now,
        b":solanum.example 005 papo CHANMODES=eIbq,k,flj,CFLPQcgimnprstz :are supported",
    );
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");

    for (mode, end, extra) in [
        (ListMode::Except, 349, ""),
        (ListMode::Invex, 347, ""),
        (ListMode::Quiet, 729, " q"),
    ] {
        let id = request(&mut session, now, mode);
        session.handle_line(
            now,
            format!(":solanum.example {end} papo #papo{extra} :End of mode list").as_bytes(),
        );
        assert!(response(&mut session, id).mode_list.unwrap().is_empty());
    }

    session.handle_line(
        now,
        b"@time=2026-10-21T12:00:00.000Z :alice!~alice@staff.example MODE #papo +eIq *!*@staff.example $a:invited *!*@noisy.example",
    );
    for (mode, mask) in [
        (ListMode::Except, "*!*@staff.example"),
        (ListMode::Invex, "$a:invited"),
        (ListMode::Quiet, "*!*@noisy.example"),
    ] {
        let entries = session
            .state()
            .channel("#papo")
            .unwrap()
            .list(mode)
            .unwrap();
        assert!(matches!(entries, [entry] if entry.mask == mask));
    }
}

#[test]
fn unreal_account_extbans_compare_by_account_identity() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(
        now,
        b":irc.unreal.example 005 papo ACCOUNTEXTBAN=account,a CHANMODES=beI,fkL,lFH,cdimnprstzCDGKMNOPQRSTVZ EXTBAN=~,acfijmnpqrtACFGOST :are supported",
    );
    let state = session.state();
    let account = state.casemapping().mask("~a:Alice");
    assert!(state.same_list_entry(&account, "~account:alice"));
    assert!(!state.same_list_entry(&account, "~account:bob"));
    assert!(!state.same_list_entry(&account, "*!*@staff.example"));
    assert!(state.same_list_entry(&state.casemapping().mask("Bob"), "bob!*@*"));
    assert!(!state.same_list_entry(&state.casemapping().mask("Bob"), "alice!*@*"));
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
    session.handle_line(now, b":solanum.example 001 papo :Welcome to Solanum");
    session.handle_line(now, b":solanum.example 422 papo :MOTD File is missing");
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    while session.poll_event().is_some() {}

    let id = request(&mut session, now, ListMode::Ban);
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
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
    let id = request(&mut bouncer, now, ListMode::Ban);
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
    let id = request(&mut nonmember, now, ListMode::Ban);
    nonmember.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    assert_eq!(response(&mut nonmember, id).mode_list, Some(Vec::new()));
    assert!(nonmember.state().channel("#papo").is_none());
}

#[test]
fn a_refusal_invalidates_a_previously_fetched_list() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let id = request(&mut session, now, ListMode::Ban);
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

    let id = request(&mut session, now, ListMode::Ban);
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

#[test]
fn live_changes_during_a_fetch_do_not_duplicate_the_snapshot() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(
        now,
        b":ergo.test 005 papo CHANMODES=beI,k,l,imnst :are supported",
    );
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let id = request(&mut session, now, ListMode::Ban);
    session.handle_line(now, b":ergo.test 367 papo #papo bob!*@* oper 1761048000");
    session.handle_line(now, b":alice!~alice@staff.example MODE #papo +b bob");
    session.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");

    let entries = response(&mut session, id).mode_list.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        session
            .state()
            .channel("#papo")
            .unwrap()
            .list(ListMode::Ban),
        Some(entries.as_slice())
    );
}

#[test]
fn timeout_and_disconnect_invalidate_tracked_lists() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let id = request(&mut session, now, ListMode::Ban);
    session.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    _ = response(&mut session, id);

    let id = request(&mut session, now, ListMode::Ban);
    while session.poll_transmit().is_some() {}
    session.handle_line(
        now,
        b":ergo.test 367 papo #papo *!*@gateway.example oper 1761048000",
    );
    session.handle_timeout(session.poll_timeout().unwrap() + Duration::from_secs(31));
    assert!(std::iter::from_fn(|| session.poll_event()).any(
        |event| matches!(event, Event::RequestTimedOut { id: timed_out, .. } if timed_out == id)
    ));
    assert_eq!(
        session
            .state()
            .channel("#papo")
            .unwrap()
            .list(ListMode::Ban),
        None
    );

    let mut session = Session::new(Config::default());
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    let id = request(&mut session, now, ListMode::Ban);
    session.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    _ = response(&mut session, id);
    session.handle_disconnected(now);
    assert!(session.state().channel("#papo").is_none());
}

#[test]
fn labelled_timeout_and_overflow_forget_the_list_and_its_label() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .wanted_caps(["labeled-response"])
            .request_timeout(Duration::from_secs(1))
            .rate_limit(papo_core::RateLimit::unlimited()),
    );
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :labeled-response");
    session.handle_line(now, b"CAP * ACK :labeled-response");
    session.handle_line(now, b":ergo.test 001 papo :Welcome");
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
    session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
    while session.poll_event().is_some() {}

    let id = request(&mut session, now, ListMode::Ban);
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"@label=0 MODE #papo b\r\n"
    );
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

    let overflowed = request(&mut session, now, ListMode::Ban);
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"@label=1 MODE #papo b\r\n"
    );
    for index in 0..=1000 {
        session.handle_line(
            now,
            format!(":ergo.test 367 papo #papo *!*@host{index}.example oper 1761048000").as_bytes(),
        );
    }
    assert!(std::iter::from_fn(|| session.poll_event()).any(
        |event| matches!(event, Event::Response(response) if response.id == overflowed && response.state_limit == Some(papo_core::StateLimit::ListEntries))
    ));
    session.handle_line(now, b"@label=1 ACK");
    assert!(
        !std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::Response(response) if response.id == overflowed))
    );

    let id = request(&mut session, now, ListMode::Ban);
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"@label=2 MODE #papo b\r\n"
    );
    session.handle_line(now, b":ergo.test 368 papo #papo :End of channel ban list");
    _ = response(&mut session, id);
    let timed_out = request(&mut session, now, ListMode::Ban);
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"@label=3 MODE #papo b\r\n"
    );
    session.handle_timeout(now + Duration::from_secs(2));
    assert!(
        std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::RequestTimedOut { id, .. } if id == timed_out))
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
