//! Transcript coverage for WHO-proven identities of users the client does not share a channel with.

use std::time::Instant;

use papo_core::{Account, Config, ProofFailure, Proven, RateLimit, Session};
use papo_proto::Line;

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

fn registered() -> Session {
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited()),
    );
    session.handle_connected(Instant::now());
    server(&mut session, ":ergo.test 001 papo :Welcome");
    server(
        &mut session,
        ":ergo.test 005 papo CASEMAPPING=rfc1459 MONITOR=100 :are supported",
    );
    server(&mut session, ":ergo.test 422 papo :MOTD missing");
    while session.poll_transmit().is_some() {}
    session
}

fn prove(session: &mut Session, nick: &str, account: &str) -> Result<Proven, ProofFailure> {
    let casemapping = session.state().casemapping();
    let expected = if account == "0" {
        Account::LoggedOut
    } else {
        Account::LoggedIn(casemapping.account(account))
    };
    session.__prove_who_user(
        casemapping.nick(nick),
        expected,
        &[Line::new("354").with_params(["papo", nick, account])],
    )
}

fn resolved(session: &Session, proof: &Proven) -> Option<String> {
    session
        .state()
        .__resolve_proven(proof)
        .ok()
        .map(|nick| nick.to_string())
}

#[test]
fn who_proof_accepts_a_shared_user_whose_account_is_unknown() {
    let mut session = registered();
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(&mut session, ":alice!~alice@cloak.example JOIN #review");
    assert_eq!(
        session.state().user("alice").map(papo_core::User::account),
        Some(&Account::Unknown)
    );

    let proof = prove(&mut session, "alice", "alice-account").unwrap();
    assert_eq!(resolved(&session, &proof).as_deref(), Some("alice"));
}

#[test]
fn who_proofs_of_unshared_users_resolve_to_their_own_nick() {
    let mut session = registered();
    let alice = prove(&mut session, "alice", "0").unwrap();
    let bob = prove(&mut session, "bob", "0").unwrap();

    assert_eq!(resolved(&session, &alice).as_deref(), Some("alice"));
    assert_eq!(resolved(&session, &bob).as_deref(), Some("bob"));
}

#[test]
fn who_proof_table_keeps_the_newest_thousand_identities() {
    let mut session = registered();
    let first = prove(&mut session, "reviewer0", "0").unwrap();
    let second = prove(&mut session, "reviewer1", "0").unwrap();
    for index in 2..1_000 {
        prove(&mut session, &format!("reviewer{index}"), "0").unwrap();
    }
    assert_eq!(resolved(&session, &first).as_deref(), Some("reviewer0"));

    prove(&mut session, "reviewer1000", "0").unwrap();
    assert_eq!(resolved(&session, &first), None);
    assert_eq!(resolved(&session, &second).as_deref(), Some("reviewer1"));
}
