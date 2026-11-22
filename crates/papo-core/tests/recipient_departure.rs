//! Transcript coverage for bounded departed recipient identities.

use std::time::Instant;

use papo_core::{AccountStatus, Config, RecipientStatus, Session};

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

fn depart_reviewers(session: &mut Session, indices: std::ops::Range<usize>) {
    for index in indices {
        server(
            session,
            &format!(":reviewer{index}!~review@users.example JOIN #review"),
        );
        server(
            session,
            &format!(":reviewer{index}!~review@users.example QUIT :Review complete"),
        );
    }
}

fn resolve_after_forgotten_departure(guard_account: Option<&str>, rejoin: &str) -> RecipientStatus {
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, ":papo!~papo@localhost JOIN #review * :Papo");
    server(
        &mut session,
        ":alice!~alice@cloak.example JOIN #review alice-account :Alice",
    );
    let account = guard_account.map_or(AccountStatus::Unknown, |account| {
        AccountStatus::LoggedIn(session.state().casemapping().account(account))
    });
    let guard = session
        .state()
        .__capture_recipient(session.state().casemapping().nick("alice"), account)
        .unwrap();
    server(
        &mut session,
        ":alice!~alice@cloak.example QUIT :Quit: rebooting",
    );
    depart_reviewers(&mut session, 0..1000);
    server(&mut session, rejoin);
    session.state().__resolve_recipient(&guard)
}

#[test]
fn forgotten_departure_leaves_a_logged_in_guard_unknown_beside_an_unknown_account() {
    assert_eq!(
        resolve_after_forgotten_departure(
            Some("alice-account"),
            ":alice!~alice@laptop.example JOIN #review",
        ),
        RecipientStatus::Unknown
    );
}

#[test]
fn forgotten_departure_never_resolves_a_logged_in_guard_to_a_known_account_holder() {
    assert_eq!(
        resolve_after_forgotten_departure(
            Some("alice-account"),
            ":alice!~alice@laptop.example JOIN #review alice-account :Alice",
        ),
        RecipientStatus::Gone
    );
}

#[test]
fn forgotten_departure_never_resolves_an_accountless_guard_to_the_next_holder() {
    assert_eq!(
        resolve_after_forgotten_departure(None, ":alice!~alice@laptop.example JOIN #review"),
        RecipientStatus::Gone
    );
}

#[test]
fn departed_identity_history_is_bounded_and_fails_closed() {
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(&mut session, ":alice!~alice@cloak.example JOIN #review");
    let guard = session
        .state()
        .__capture_recipient(
            session.state().casemapping().nick("alice"),
            AccountStatus::Unknown,
        )
        .unwrap();
    server(
        &mut session,
        ":alice!~alice@cloak.example QUIT :Client Quit",
    );

    depart_reviewers(&mut session, 0..999);
    assert_eq!(
        session.state().__resolve_recipient(&guard),
        RecipientStatus::Gone
    );

    server(
        &mut session,
        ":final-reviewer!~review@users.example JOIN #review",
    );
    server(
        &mut session,
        ":final-reviewer!~review@users.example QUIT :Review complete",
    );
    assert_eq!(
        session.state().__resolve_recipient(&guard),
        RecipientStatus::Unknown
    );
}
