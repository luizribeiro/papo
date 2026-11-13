//! Transcript coverage for guarded direct-recipient identity resolution.

use std::time::{Duration, Instant};

use papo_core::{Account, Config, Event, PeerToken, ProofFailure, RecipientStatus, Session};
use papo_proto::{Line, Origin};

fn server(session: &mut Session, now: Instant, line: &str) {
    session.handle_line(now, line.as_bytes());
}

fn peer(session: &Session, nick: &str) -> PeerToken {
    session.state().__peer_token_for(nick)
}

fn session_with_origin(origin: u64) -> Session {
    let mut config = Config::default().nick("papo");
    config.origin = Origin::__new(origin);
    Session::new(config)
}

fn alice_event(session: &mut Session, now: Instant) -> papo_core::ReplyToken {
    server(session, now, ":papo!~papo@localhost JOIN #review");
    server(
        session,
        now + Duration::from_millis(1),
        ":alice!~alice@cloak.example JOIN #review alice-account :Alice Example",
    );
    server(
        session,
        now + Duration::from_millis(2),
        "@msgid=ErgoProof1;account=alice-account :alice!~alice@cloak.example PRIVMSG #review :release approved",
    );
    std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::Line(line) if line.as_ref().command_is("PRIVMSG") => Some(line.__reply_token()),
            _ => None,
        })
        .expect("Ergo message event carries a reply token")
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
    let token = session.state().__reply_token();
    assert!(!token.__is_known());
    let result = session.state().__prove_event_user(
        token,
        session.state().casemapping().nick("alice"),
        Account::LoggedIn(session.state().casemapping().account("alice-account")),
    );
    assert_eq!(result.unwrap_err(), ProofFailure::MissingEvidence);
}

#[test]
fn event_proof_adapters_preserve_a_concrete_recipient() {
    let now = Instant::now();
    let mut session = session_with_origin(73);
    let token = alice_event(&mut session, now);
    assert!(token.__is_known());
    let nick = Origin::__new(73).__nick(session.state().casemapping(), "alice");
    let account = Account::LoggedIn(
        Origin::__new(73).__account(session.state().casemapping(), "alice-account"),
    );

    let proof = session
        .__prove_event_user(token, nick.clone(), account.clone())
        .expect("the event proves Alice's identity");
    assert!(proof.__recipient().is_some());
}

#[test]
fn event_proof_rejects_a_foreign_nickname_with_a_local_token() {
    let now = Instant::now();
    let mut session = session_with_origin(73);
    let token = alice_event(&mut session, now);
    let foreign_nick = Origin::__new(91).__nick(session.state().casemapping(), "alice");
    let account = Account::LoggedIn(
        Origin::__new(73).__account(session.state().casemapping(), "alice-account"),
    );

    assert_eq!(
        session
            .state()
            .__prove_event_user(token, foreign_nick, account)
            .unwrap_err(),
        ProofFailure::ForeignReference
    );
}

#[test]
fn who_proof_rejects_a_foreign_nickname() {
    let now = Instant::now();
    let mut session = session_with_origin(73);
    _ = alice_event(&mut session, now);
    let foreign_nick = Origin::__new(91).__nick(session.state().casemapping(), "alice");
    let account = Account::LoggedIn(
        Origin::__new(73).__account(session.state().casemapping(), "alice-account"),
    );
    let row = Line::new("354").with_params(["papo", "alice", "alice-account"]);

    assert_eq!(
        session
            .__prove_who_user(foreign_nick, account, &[row])
            .unwrap_err(),
        ProofFailure::ForeignReference
    );
}

#[test]
fn who_proof_rejects_another_users_row() {
    let now = Instant::now();
    let mut session = session_with_origin(73);
    _ = alice_event(&mut session, now);
    let nick = Origin::__new(73).__nick(session.state().casemapping(), "alice");
    let account = Account::LoggedIn(
        Origin::__new(73).__account(session.state().casemapping(), "alice-account"),
    );
    let mallory_row = Line::new("354").with_params(["papo", "mallory", "alice-account"]);

    assert_eq!(
        session
            .__prove_who_user(nick, account, &[mallory_row])
            .unwrap_err(),
        ProofFailure::MissingEvidence
    );
}
