//! Regression coverage for proof-backed person requests.

use std::time::Instant;

use papo_core::{Account, Config, PersonCommand, RateLimit, Session};
use papo_proto::Line;

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

#[test]
fn person_request_retargets_the_proven_user() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited()),
    );
    session.handle_connected(now);
    server(&mut session, ":ergo.test CAP * LS :labeled-response");
    server(&mut session, ":ergo.test CAP * ACK :labeled-response");
    while session.poll_transmit().is_some() {}
    server(&mut session, ":ergo.test 001 papo :Welcome");
    server(&mut session, ":ergo.test 422 papo :MOTD missing");
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(
        &mut session,
        ":alice!a@alice.example JOIN #review alice-account :Alice",
    );
    while session.poll_event().is_some() {}

    let nick = session.state().casemapping().nick("alice");
    let account = Account::LoggedIn(session.state().casemapping().account("alice-account"));
    let row = Line::new("354").with_params(["papo", "alice", "alice-account"]);
    let proof = session.__prove_who_user(nick, account, &[row]).unwrap();
    let command = PersonCommand::__new(Line::new("WHOIS").param("template")).unwrap();
    session.request_person(now, proof, command).unwrap();
    server(&mut session, ":alice!a@alice.example NICK alicia");

    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"@label=0 WHOIS alicia\r\n"
    );
}
