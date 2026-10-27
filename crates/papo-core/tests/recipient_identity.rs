//! Transcript coverage for guarded direct-recipient identity resolution.

use std::time::Instant;

use papo_core::{Account, Config, RecipientGuard, RecipientStatus, Session};

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

#[test]
fn recipient_identity_follows_rename_and_rejects_account_change() {
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, ":papo!~papo@localhost JOIN #review");
    server(
        &mut session,
        ":alice!~alice@cloak.example JOIN #review alice :Alice Example",
    );
    let guard = RecipientGuard::__new(
        session.state(),
        session.state().casemapping().nick("alice"),
        Account::LoggedIn(session.state().casemapping().account("alice")),
    );

    server(&mut session, ":alice!~alice@cloak.example NICK alicia");
    assert!(matches!(
        session.state().__resolve_recipient(&guard),
        RecipientStatus::Current(nick) if nick == "alicia"
    ));

    server(&mut session, ":alicia!~alice@cloak.example ACCOUNT mallory");
    assert_eq!(
        session.state().__resolve_recipient(&guard),
        RecipientStatus::Unknown
    );
}
