//! Transcript coverage for receipt-derived channel identity.

use std::time::Instant;

use papo_core::{Config, Event, Session, TargetGuard, TextKind};

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
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
