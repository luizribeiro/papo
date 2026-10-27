//! Transcript coverage for bounded departed recipient identities.

use std::time::Instant;

use papo_core::{Account, Config, Session};

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

#[test]
fn departed_identity_history_is_bounded_and_fails_closed() {
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(&mut session, ":alice!~alice@cloak.example JOIN #review");
    let guard = session.__direct_acquire(
        "alice",
        Some("~alice"),
        Some("cloak.example"),
        Account::LoggedOut,
    );
    server(
        &mut session,
        ":alice!~alice@cloak.example QUIT :Client Quit",
    );

    for index in 0..999 {
        server(
            &mut session,
            &format!(":reviewer{index}!~review@users.example JOIN #review"),
        );
        server(
            &mut session,
            &format!(":reviewer{index}!~review@users.example QUIT :Review complete"),
        );
    }
    assert_eq!(
        session.state().__resolve_recipient(&guard),
        papo_core::RecipientStatus::Gone
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
        papo_core::RecipientStatus::Unknown
    );
}
