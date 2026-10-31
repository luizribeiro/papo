//! Transcript coverage for guarded direct-recipient identity resolution.

use std::time::{Duration, Instant};

use papo_core::{Account, Config, RecipientGuard, RecipientStatus, ReplyToken, Session};

fn server(session: &mut Session, now: Instant, line: &str) {
    session.handle_line(now, line.as_bytes());
}

#[test]
fn recipient_identity_follows_rename_and_rejects_account_change() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, now, ":papo!~papo@localhost JOIN #review");
    server(
        &mut session,
        now + Duration::from_millis(1),
        ":alice!~alice@cloak.example JOIN #review alice :Alice Example",
    );
    let guard = RecipientGuard::__new(
        session.state(),
        session.state().casemapping().nick("alice"),
        Account::LoggedIn(session.state().casemapping().account("alice")),
    );

    server(
        &mut session,
        now + Duration::from_millis(2),
        ":alice!~alice@cloak.example NICK alicia",
    );
    assert!(matches!(
        session.state().__resolve_recipient(&guard),
        RecipientStatus::Current(nick) if nick == "alicia"
    ));

    server(
        &mut session,
        now + Duration::from_millis(3),
        ":alicia!~alice@cloak.example ACCOUNT mallory",
    );
    assert_eq!(
        session.state().__resolve_recipient(&guard),
        RecipientStatus::Unknown
    );
}

#[test]
fn account_guard_requires_proof_for_unknown_or_absent_nick() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, now, ":papo!~papo@localhost JOIN #review");
    server(
        &mut session,
        now + Duration::from_millis(1),
        ":alice!~alice@cloak.example JOIN #review alice-account :Alice Example",
    );
    let nick = session.state().casemapping().nick("alice");
    let guard = RecipientGuard::__from_parts(
        nick.clone(),
        Account::LoggedIn(session.state().casemapping().account("alice-account")),
        ReplyToken::__unknown(nick.origin()),
    );

    server(
        &mut session,
        now + Duration::from_millis(2),
        ":alice!~alice@cloak.example ACCOUNT mallory-account",
    );
    assert_eq!(
        session.state().__resolve_recipient(&guard),
        RecipientStatus::Gone
    );
    server(
        &mut session,
        now + Duration::from_millis(3),
        ":alice!~alice@cloak.example ACCOUNT *",
    );
    assert_eq!(
        session.state().__resolve_recipient(&guard),
        RecipientStatus::Gone
    );
    server(
        &mut session,
        now + Duration::from_millis(4),
        ":alice!~alice@cloak.example QUIT :Client exited",
    );
    assert_eq!(
        session.state().__resolve_recipient(&guard),
        RecipientStatus::Unknown
    );
    server(
        &mut session,
        now + Duration::from_millis(5),
        ":alice!~alice@new-cloak.example JOIN #review",
    );
    assert_eq!(
        session.state().__resolve_recipient(&guard),
        RecipientStatus::Unknown
    );
}
