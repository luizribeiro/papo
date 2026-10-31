//! Transcript coverage for receipt-derived channel identity.

use std::time::{Duration, Instant, UNIX_EPOCH};

use papo_core::{
    ChannelGuard, Config, Event, RateLimit, RequestId, Session, TargetGuard, TextKind,
};
use papo_proto::Casemapping;

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

fn labelled_session(now: Instant) -> Session {
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited()),
    );
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :labeled-response");
    session.handle_line(now, b"CAP * ACK :labeled-response");
    while session.poll_transmit().is_some() {}
    session
}

fn pending_join_guard(session: &mut Session, now: Instant) -> (RequestId, ChannelGuard) {
    let id = session.join(now, "#review", None).unwrap();
    assert!(session.poll_transmit().is_some());
    server(
        session,
        "@msgid=Ergo8ZZ5 :alice!~alice@cloak.example PRIVMSG #review :!ping",
    );
    let guard = std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::Line(line) => line.__channel_guard().cloned(),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        session.state().__resolve_channel(&guard).as_deref(),
        Some("#review")
    );
    (id, guard)
}

fn assert_pending_reply_refused(session: &mut Session, guard: ChannelGuard) {
    assert_eq!(session.state().__resolve_channel(&guard), None);
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
    session
        .send_target_guarded_text_detailed(
            Instant::now(),
            TextKind::Privmsg,
            "#review",
            "pong",
            &[],
            Some(TargetGuard::Channel(guard)),
        )
        .unwrap();
    assert_eq!(session.poll_transmit(), None);
}

#[test]
fn channel_message_during_join_keeps_the_reserved_membership() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited()),
    );
    session.join(now, "#review", None).unwrap();
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"JOIN #review\r\n"
    );

    server(
        &mut session,
        "@msgid=Ergo8ZZ5 :alice!~alice@cloak.example PRIVMSG #review :!ping",
    );
    let guard = std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::Line(line) => line.__channel_guard().cloned(),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        session.state().__resolve_channel(&guard).as_deref(),
        Some("#review")
    );

    server(&mut session, ":papo!p@localhost JOIN #review");
    assert_eq!(
        session.state().__resolve_channel(&guard).as_deref(),
        Some("#review")
    );
    session
        .send_target_guarded_text_detailed(
            now,
            TextKind::Privmsg,
            "#review",
            "pong",
            &[],
            Some(TargetGuard::Channel(guard)),
        )
        .unwrap();
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"PRIVMSG #review pong\r\n"
    );
}

#[test]
fn labelled_failure_releases_the_pending_join() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let (_, guard) = pending_join_guard(&mut session, now);

    server(
        &mut session,
        "@label=0 :ergo.test FAIL JOIN ACCOUNT_REQUIRED #review :Account required",
    );

    assert_pending_reply_refused(&mut session, guard);
}

#[test]
fn unlabelled_completion_releases_the_pending_join() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited()),
    );
    let (_, guard) = pending_join_guard(&mut session, now);

    server(
        &mut session,
        ":ergo.test FAIL JOIN ACCOUNT_REQUIRED #review :Account required",
    );

    assert_pending_reply_refused(&mut session, guard);
}

#[test]
fn every_join_refusal_numeric_releases_the_pending_join() {
    for line in [
        ":ergo.test 403 papo #review :No such channel",
        ":ergo.test 405 papo #review :You have joined too many channels",
        ":ergo.test 471 papo #review :Cannot join channel (+l)",
        ":ergo.test 473 papo #review :Cannot join channel (+i)",
        ":ergo.test 474 papo #review :Cannot join channel (+b)",
        ":ergo.test 475 papo #review :Cannot join channel (+k)",
    ] {
        let now = Instant::now();
        let mut session = Session::new(
            Config::default()
                .nick("papo")
                .rate_limit(RateLimit::unlimited()),
        );
        let (_, guard) = pending_join_guard(&mut session, now);

        server(&mut session, line);

        assert_pending_reply_refused(&mut session, guard);
    }
}

#[test]
fn timeout_releases_the_pending_join() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited()),
    );
    let (_, guard) = pending_join_guard(&mut session, now);
    session.handle_timeout(now + Duration::from_secs(30));
    assert_pending_reply_refused(&mut session, guard);
}

#[test]
fn transport_cancellation_releases_the_pending_join() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited()),
    );
    let (id, guard) = pending_join_guard(&mut session, now);
    session.__cancel_request(id);
    assert_pending_reply_refused(&mut session, guard);
}

#[test]
fn own_departure_aborts_and_releases_the_pending_join() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited()),
    );
    let (_, guard) = pending_join_guard(&mut session, now);

    server(
        &mut session,
        ":papo!p@localhost PART #review :Registration required",
    );

    assert_pending_reply_refused(&mut session, guard);
}

#[test]
fn disconnect_and_connection_clear_release_pending_joins() {
    let now = Instant::now();
    let mut disconnected = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited()),
    );
    let (_, disconnected_guard) = pending_join_guard(&mut disconnected, now);
    disconnected.handle_disconnected(now);
    assert_pending_reply_refused(&mut disconnected, disconnected_guard);

    let mut cleared = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited()),
    );
    let (_, cleared_guard) = pending_join_guard(&mut cleared, now);
    cleared.handle_connected(now);
    assert_pending_reply_refused(&mut cleared, cleared_guard);
}

#[test]
fn pending_join_follows_a_casemapping_change() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .casemapping(Casemapping::Ascii)
            .rate_limit(RateLimit::unlimited()),
    );
    session.join(now, "#Review[", None).unwrap();
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"JOIN #Review[\r\n"
    );

    server(
        &mut session,
        ":ergo.test 005 papo CASEMAPPING=rfc1459 :are supported",
    );
    server(
        &mut session,
        "@msgid=Ergo91B2 :alice!~alice@cloak.example PRIVMSG #review{ :!ping",
    );
    let guard = std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::Line(line) => line.__channel_guard().cloned(),
            _ => None,
        })
        .unwrap();

    server(&mut session, ":papo!p@localhost JOIN #review{");
    assert_eq!(
        session.state().__resolve_channel(&guard).as_deref(),
        Some("#review{")
    );
}

#[test]
fn membership_guard_follows_rename_and_expires_on_part() {
    let mut session = Session::new(Config::default().nick("papo"));
    server(
        &mut session,
        ":ergo.test 005 papo PREFIX=(ov)@+ STATUSMSG=@ :are supported",
    );
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(&mut session, ":ergo.test 329 papo #review 1793098800");
    server(
        &mut session,
        ":alice!~alice@cloak.example PRIVMSG @#review :Release approved",
    );
    let guard = std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::Line(line) => line.__channel_guard().cloned(),
            _ => None,
        })
        .unwrap();
    session
        .send_target_guarded_text_detailed(
            Instant::now(),
            TextKind::Notice,
            "@#review",
            "release approved",
            &[],
            Some(TargetGuard::Channel(guard.clone())),
        )
        .unwrap();

    server(
        &mut session,
        ":alice!~alice@cloak.example RENAME #review #release :Review completed",
    );
    assert_eq!(
        session.state().__resolve_channel(&guard).as_deref(),
        Some("#release")
    );
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"NOTICE @#release :release approved\r\n"
    );
    session
        .send_target_guarded_text_detailed(
            Instant::now(),
            TextKind::Notice,
            "@#release",
            "already resolved",
            &[],
            Some(TargetGuard::Channel(guard.clone())),
        )
        .unwrap();
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"NOTICE @#release :already resolved\r\n"
    );

    let token = session
        .send_target_guarded_action_detailed(
            Instant::now(),
            "#release",
            "deployment started",
            &[],
            Some(TargetGuard::Channel(guard.clone())),
        )
        .unwrap()
        .transmit
        .unwrap();

    server(
        &mut session,
        ":papo!p@localhost PART #release :Deployment complete",
    );
    assert_eq!(session.state().__resolve_channel(&guard), None);
    assert_eq!(session.poll_transmit(), None);
    assert!(std::iter::from_fn(|| session.poll_event()).any(
        |event| matches!(event, Event::TransmitRecipientGone { token: gone, .. } if gone == token)
    ));
}

#[test]
fn creation_time_proves_a_channel_across_membership_epochs() {
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(&mut session, ":ergo.test 329 papo #review 1793098800");
    let channel = session.state().channel("#review").unwrap().name().clone();
    let guard = session.state().__channel_guard(&channel);

    server(
        &mut session,
        ":papo!p@localhost PART #review :Network maintenance",
    );
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(&mut session, ":ergo.test 329 papo #review 1793098800");
    assert_eq!(
        session.state().__resolve_channel(&guard).as_deref(),
        Some("#review")
    );

    session
        .send_target_guarded_text_detailed(
            Instant::now(),
            TextKind::Privmsg,
            "#review",
            "same channel",
            &[],
            Some(TargetGuard::Channel(guard.clone())),
        )
        .unwrap();
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"PRIVMSG #review :same channel\r\n"
    );

    server(
        &mut session,
        ":papo!p@localhost PART #review :Network maintenance",
    );
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(&mut session, ":ergo.test 329 papo #review 1793099900");
    assert_eq!(session.state().__resolve_channel(&guard), None);
    session
        .send_target_guarded_text_detailed(
            Instant::now(),
            TextKind::Privmsg,
            "#review",
            "stale channel",
            &[],
            Some(TargetGuard::Channel(guard)),
        )
        .unwrap();
    assert_eq!(session.poll_transmit(), None);
}

#[test]
#[allow(clippy::duration_suboptimal_units)]
fn reconnect_reports_a_recreated_channel_without_refusing_the_rejoin() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    for channel in ["#release", "#stable"] {
        server(&mut session, &format!(":papo!p@localhost JOIN {channel}"));
        server(
            &mut session,
            &format!(":ergo.test 329 papo {channel} 1793098800"),
        );
    }
    while session.poll_event().is_some() {}

    session.handle_disconnected(now);
    session.handle_connected(now);
    for (channel, created_at) in [("#release", 1_793_099_900), ("#stable", 1_793_098_800)] {
        server(&mut session, &format!(":papo!p@localhost JOIN {channel}"));
        server(
            &mut session,
            &format!(":ergo.test 329 papo {channel} {created_at}"),
        );
    }

    let recreated: Vec<_> = std::iter::from_fn(|| session.poll_event())
        .filter_map(|event| match event {
            Event::ChannelRecreated {
                channel,
                previous_created_at,
                created_at,
                ..
            } => Some((channel, previous_created_at, created_at)),
            _ => None,
        })
        .collect();
    assert_eq!(recreated.len(), 1);
    assert_eq!(recreated[0].0, "#release");
    assert_eq!(
        recreated[0].1,
        UNIX_EPOCH + Duration::from_secs(1_793_098_800)
    );
    assert_eq!(
        recreated[0].2,
        UNIX_EPOCH + Duration::from_secs(1_793_099_900)
    );
    assert!(session.state().channel("#release").is_some());
}
