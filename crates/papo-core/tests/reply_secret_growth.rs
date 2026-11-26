//! Server input never adds to the secrets a session keeps to mask failure replies.

use std::time::Instant;

use papo_core::{Config, RateLimit, Sasl, Secret, Session, test_support::reply_secret_count};
use papo_proto::Line;

const SERVER_PASSWORD: &str = "server-password-7193";
const CHANNEL_KEY: &str = "staff-key-4821";

fn feed(session: &mut Session, now: Instant, line: &str) {
    session.handle_line(now, line.as_bytes());
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
}

fn assert_never_grows(
    session: &mut Session,
    now: Instant,
    lines: impl IntoIterator<Item = String>,
) {
    let kept = reply_secret_count(session);
    for line in lines {
        feed(session, now, &line);
        assert!(reply_secret_count(session) <= kept, "{line}");
    }
}

fn logging_in(now: Instant) -> Session {
    let sasl = Sasl::plain("papo", Secret::new("services-password-4821")).optional();
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .password(Secret::new(SERVER_PASSWORD))
            .sasl(sasl)
            .connection_security(papo_core::sts::ConnectionSecurity::Secure)
            .rate_limit(RateLimit::unlimited()),
    );
    session.handle_connected(now);
    for line in [
        ":ergo.test CAP * LS :sasl=PLAIN",
        ":ergo.test CAP * ACK :sasl",
        "AUTHENTICATE +",
    ] {
        feed(&mut session, now, line);
    }
    session
}

fn restarted_logins() -> impl Iterator<Item = String> {
    (0..300).flat_map(|_| {
        [
            ":ergo.test 904 papo :SASL authentication failed",
            ":ergo.test CAP * DEL :sasl",
            ":ergo.test CAP * NEW :sasl=PLAIN",
            ":ergo.test CAP * ACK :sasl",
            "AUTHENTICATE +",
        ]
        .map(str::to_owned)
    })
}

fn every_numeric() -> impl Iterator<Item = String> {
    (0..=999).flat_map(|numeric| {
        [
            String::new(),
            " papo".to_owned(),
            " papo #staff :Cannot join channel (+k)".to_owned(),
            " papo #staff other-key-0731 :Key rejected".to_owned(),
            format!(" papo #staff {CHANNEL_KEY} :Key rejected"),
        ]
        .map(|params| format!(":ergo.test {numeric:03}{params}"))
    })
}

const COMMANDS: &[&str] = &[
    ":ergo.test FAIL JOIN BAD_CHANNEL_KEY #staff other-key-0731 :Invalid channel key",
    ":ergo.test WARN REGISTER WEAK_PASSWORD papo other-password-0731 :Easy to guess",
    ":ergo.test NOTE OPER OPER_MOTD papo :Operator privileges are logged",
    ":ergo.test NOTICE papo :Your key other-key-0731 was rejected",
    ":alice!~alice@users.example NOTICE papo :other-key-0731",
    ":alice!~alice@users.example PRIVMSG #staff :the key is other-key-0731",
    ":ergo.test CAP papo LS :sasl=PLAIN,SCRAM-SHA-256 server-time",
    ":ergo.test CAP papo NEW :draft/account-registration=before-connect",
    ":ergo.test CAP papo DEL :draft/account-registration",
    ":ergo.test CAP papo ACK :server-time",
    ":ergo.test CAP papo NAK :draft/metadata-2",
    "AUTHENTICATE +",
    "AUTHENTICATE cj1meWtvK0d3VTF3c2RmV0pZd1d2T2NBPT0sdj1yWHE=",
    ":alice!~alice@users.example JOIN #staff other-key-0731",
    ":alice!~alice@users.example MODE #staff +k other-key-0731",
    ":ergo.test MODE #staff +k other-key-0732",
    ":alice!~alice@users.example PASS other-password-0731",
    ":alice!~alice@users.example OPER admin other-password-0731",
    ":ergo.test REGISTER SUCCESS papo :Account created",
    ":ergo.test VERIFY SUCCESS papo :Account verified",
    ":ergo.test WEBIRC other-password-0731 gateway gw.example 192.0.2.7",
    ":ergo.test BATCH +hist chathistory #staff",
    "@batch=hist :alice!~alice@users.example PRIVMSG #staff :other-key-0731",
    "@batch=hist :ergo.test FAIL JOIN BAD_CHANNEL_KEY #staff other-key-0733 :Invalid",
    ":ergo.test BATCH -hist",
    ":ergo.test BATCH +open labeled-response",
    "@batch=open :ergo.test 475 papo #staff :Cannot join channel (+k) other-key-0734",
    ":ergo.test ERROR :Closing Link: papo (other-password-0731)",
];

#[test]
fn server_lines_never_add_a_kept_secret() {
    let now = Instant::now();
    let mut session = logging_in(now);
    let kept = reply_secret_count(&session);
    assert_eq!(
        kept, 3,
        "server password, SASL password and its payload line"
    );
    assert_never_grows(&mut session, now, restarted_logins());
    for line in [
        ":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo",
        ":ergo.test 005 papo CHANMODES=beI,k,l,imnst PREFIX=(qaohv)~&@%+ :are supported",
        ":ergo.test 422 papo :MOTD File is missing",
    ] {
        feed(&mut session, now, line);
    }
    session
        .send(
            now,
            Line::new("JOIN")
                .param("#staff")
                .param(Secret::new(CHANNEL_KEY)),
        )
        .unwrap();
    assert!(session.poll_transmit().is_some());
    let kept = reply_secret_count(&session);
    assert_eq!(kept, 4, "the channel key the application sent");
    assert_never_grows(&mut session, now, every_numeric());
    assert_never_grows(
        &mut session,
        now,
        COMMANDS.iter().map(|line| (*line).to_owned()),
    );
    assert_eq!(reply_secret_count(&session), kept);
}
