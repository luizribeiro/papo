//! Transcript coverage for receipt-derived channel identity.

use std::time::Instant;

use papo_core::{Config, Session};

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

#[test]
fn membership_guard_follows_rename_and_expires_on_part() {
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(&mut session, ":ergo.test 329 papo #review 1793098800");
    let channel = session.state().channel("#review").unwrap().name().clone();
    let guard = session.state().__channel_guard(&channel);

    server(
        &mut session,
        ":alice!~alice@cloak.example RENAME #review #release :Review completed",
    );
    assert_eq!(
        session.state().__resolve_channel(&guard).as_deref(),
        Some("#release")
    );

    server(
        &mut session,
        ":papo!p@localhost PART #release :Deployment complete",
    );
    assert_eq!(session.state().__resolve_channel(&guard), None);
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

    server(
        &mut session,
        ":papo!p@localhost PART #review :Network maintenance",
    );
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(&mut session, ":ergo.test 329 papo #review 1793099900");
    assert_eq!(session.state().__resolve_channel(&guard), None);
}
