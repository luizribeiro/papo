//! Transcript coverage for guarded direct-recipient identity resolution.

use std::time::{Duration, Instant};

use papo_core::{Account, Config, PeerToken, ProofFailure, RecipientStatus, Session};

fn server(session: &mut Session, now: Instant, line: &str) {
    session.handle_line(now, line.as_bytes());
}

fn peer(session: &Session, nick: &str) -> PeerToken {
    session.state().__peer_token_for(nick)
}

#[test]
fn peer_proof_follows_nick_and_account_knowledge() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, now, ":papo!~papo@localhost JOIN #review");
    server(
        &mut session,
        now + Duration::from_millis(1),
        ":alice!~alice@cloak.example JOIN #review",
    );
    let unverified = peer(&session, "alice");
    assert!(unverified.__is_known());

    server(
        &mut session,
        now + Duration::from_millis(2),
        ":alice!~alice@cloak.example NICK alicia",
    );
    assert_eq!(peer(&session, "alicia"), unverified);

    server(
        &mut session,
        now + Duration::from_millis(3),
        ":alicia!~alice@cloak.example ACCOUNT alice-account",
    );
    let logged_in = peer(&session, "alicia");
    assert_eq!(logged_in, unverified);
    server(
        &mut session,
        now + Duration::from_millis(4),
        ":alicia!~alice@cloak.example ACCOUNT *",
    );
    let logged_out = peer(&session, "alicia");
    assert_eq!(logged_out, logged_in);
    assert_eq!(logged_out, unverified);

    server(
        &mut session,
        now + Duration::from_millis(5),
        ":alicia!~alice@cloak.example QUIT :Client exited",
    );
    server(
        &mut session,
        now + Duration::from_millis(6),
        ":alicia!~other@new.example JOIN #review mallory :Different person",
    );
    assert_ne!(peer(&session, "alicia"), unverified);
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
    let guard = session
        .state()
        .__capture_recipient(
            session.state().casemapping().nick("alice"),
            Account::LoggedIn(session.state().casemapping().account("alice")),
        )
        .unwrap();

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
fn event_proof_requires_a_concrete_identity() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, now, ":papo!~papo@localhost JOIN #review");
    server(
        &mut session,
        now + Duration::from_millis(1),
        ":alice!~alice@cloak.example JOIN #review alice-account :Alice Example",
    );
    let result = session.state().__prove_event_user(
        session.state().__reply_token(),
        session.state().casemapping().nick("alice"),
        Account::LoggedIn(session.state().casemapping().account("alice-account")),
    );
    assert_eq!(result.unwrap_err(), ProofFailure::MissingEvidence);
}
