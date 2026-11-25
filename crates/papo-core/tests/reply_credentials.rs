//! Every credential kind against every failure reply that could repeat it.

use std::time::Instant;

use papo_core::{Config, Event, RateLimit, Sasl, Secret, Session, WebIrc};
use papo_proto::Line;

const CREDENTIAL: &str = "kestrel-vault-7193";
const ACCOUNT_CAP: &str = "draft/account-registration";

/// Each failure-class command and numeric family, as Ergo, Solanum and soju word them.
const REPLIES: &[&str] = &[
    "ERROR :Closing Link: papo (Bad password)",
    "ERROR papo :Closing Link",
    "FAIL JOIN BAD_CHANNEL_KEY #staff :Invalid channel key",
    "WARN REGISTER WEAK_PASSWORD papo :The password is easy to guess",
    "NOTE OPER OPER_MOTD papo :Operator privileges are logged",
    "263 papo JOIN :Please wait a while and try again.",
    "464 papo :Password incorrect",
    "475 papo #staff :Cannot join channel (+k)",
    "525 papo #staff :Key is not well-formed",
    "691 papo :STARTTLS failed (Wrong TLS state)",
    "696 papo #staff k * :Invalid mode k parameter",
    "698 papo #staff b *!*@gateway.example :Channel list does not contain that entry",
    "723 papo rehash :Insufficient oper privileges",
    "734 papo 100 alice,ferris :Monitor list is full",
    "902 papo :You must use a nick assigned to you",
    "904 papo :SASL authentication failed",
    "905 papo :SASL message too long",
    "906 papo :SASL authentication aborted",
    "907 papo :You have already authenticated using SASL",
];

const SOURCES: &[&str] = &["", ":ergo.test ", ":alice!~alice@users.example "];

fn config() -> Config {
    Config::default()
        .nick("papo")
        .connection_security(papo_core::sts::ConnectionSecurity::Secure)
        .rate_limit(RateLimit::unlimited())
}

fn connecting(config: Config, caps: &str, now: Instant) -> Session {
    let mut session = Session::new(config);
    session.handle_connected(now);
    for line in [
        format!(":ergo.test CAP * LS :{caps}"),
        format!(":ergo.test CAP * ACK :{caps}"),
    ] {
        session.handle_line(now, line.as_bytes());
    }
    session
}

fn registered(config: Config, caps: &str, now: Instant) -> Session {
    let mut session = connecting(config, caps, now);
    session.handle_line(
        now,
        b":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo",
    );
    session.handle_line(now, b":ergo.test 422 papo :MOTD File is missing");
    session
}

fn authenticating(sasl: Sasl, mechanism: &str, now: Instant) -> Session {
    let mut session = connecting(config().sasl(sasl), &format!("sasl={mechanism}"), now);
    session.handle_line(now, b"AUTHENTICATE +");
    session
}

/// One session per credential kind papo tracks, each with the credential already sent.
fn sessions(now: Instant) -> Vec<(&'static str, Session)> {
    let secret = || Secret::new(CREDENTIAL);
    let keyed = |line: Line| {
        let mut session = registered(config(), "multi-prefix", now);
        session.send(now, line.param(secret())).unwrap();
        session
    };
    let mut oper = registered(config(), "multi-prefix", now);
    oper.oper(now, "admin", &secret()).unwrap();
    let account = || registered(config().wanted_caps([ACCOUNT_CAP]), ACCOUNT_CAP, now);
    let (mut register, mut verify) = (account(), account());
    register.create_account(now, None, None, secret()).unwrap();
    verify.verify(now, "alice", secret()).unwrap();
    let webirc = WebIrc {
        password: secret(),
        gateway: "papo-gateway".into(),
        client_ip: "192.0.2.42".parse().unwrap(),
        client_hostname: None,
        options: Vec::new(),
    };
    let password = connecting(config().password(secret()), "multi-prefix", now);
    let gateway = connecting(config().webirc(webirc), "multi-prefix", now);
    let plain = authenticating(Sasl::plain("alice", secret()), "PLAIN", now);
    let bearer = authenticating(Sasl::oauth_bearer(secret()), "OAUTHBEARER", now);
    let mut sessions = vec![
        ("JOIN", keyed(Line::new("JOIN").param("#staff"))),
        ("MODE", keyed(Line::new("MODE").param("#staff").param("+k"))),
        ("OPER", oper),
        ("PASS", password),
        ("REGISTER", register),
        ("VERIFY", verify),
        ("WEBIRC", gateway),
        ("PLAIN", plain),
        ("OAUTHBEARER", bearer),
    ];
    for (_, session) in &mut sessions {
        while session.poll_transmit().is_some() {}
        while session.poll_event().is_some() {}
    }
    sessions
}

/// `reply` with the parameter at `position` replaced by the credential.
fn repeating(reply: &str, position: usize) -> Option<String> {
    let (head, text) = reply.split_once(" :")?;
    let mut words: Vec<_> = head.split(' ').chain([text]).collect();
    *words.get_mut(position + 1)? = CREDENTIAL;
    let text = words.pop()?;
    Some(format!("{} :{text}", words.join(" ")))
}

#[test]
fn no_failure_reply_shows_a_credential_it_repeats() {
    let now = Instant::now();
    for (kind, mut session) in sessions(now) {
        let refusal = format!(":ergo.test 404 papo #staff :Cannot send to channel: {CREDENTIAL}");
        session.handle_line(now, refusal.as_bytes());
        let shown = format!("{:?}", session.poll_event());
        assert!(
            shown.contains(": <redacted>"),
            "{kind} is not tracked: {shown}"
        );
    }
    let mut played = 0;
    for reply in REPLIES {
        for line in (0..).map_while(|position| repeating(reply, position)) {
            for source in SOURCES {
                let wire = format!("@time=2026-11-24T10:02:11.482Z {source}{line}");
                for (kind, mut session) in sessions(now) {
                    session.handle_line(now, wire.as_bytes());
                    let errors = |event: &Event| match event {
                        Event::Response(response) => format!("{:?}", response.error()),
                        _ => String::new(),
                    };
                    let kept = format!("{session:?}{:?}", session.state());
                    let shown = std::iter::from_fn(|| session.poll_event())
                        .map(|event| format!("{event:?}{}", errors(&event)))
                        .chain([kept])
                        .collect::<String>();
                    assert!(!shown.contains(CREDENTIAL), "{kind} after {wire}: {shown}");
                    played += 1;
                }
            }
        }
    }
    assert_eq!(played, 55 * SOURCES.len() * 9);
}
