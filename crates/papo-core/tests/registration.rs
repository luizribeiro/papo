//! Transcript and deadline tests for connection registration.

mod support;

use std::time::{Duration, Instant};

use papo_core::{Config, Event, Session};
use papo_proto::Isupport;
use support::Transcript;

fn line(session: &mut Session) -> String {
    String::from_utf8(session.poll_transmit().expect("client line"))
        .unwrap()
        .trim_end()
        .to_owned()
}

#[test]
fn sends_registration_in_protocol_order() {
    let now = Instant::now();
    let config = Config::default()
        .nick("emily")
        .username("em")
        .realname("Emily Example")
        .password("correct horse battery staple");
    let mut session = Session::new(config);

    session.handle_connected(now);

    assert_eq!(line(&mut session), "CAP LS 302");
    assert_eq!(line(&mut session), "PASS :correct horse battery staple");
    assert_eq!(line(&mut session), "NICK emily");
    assert_eq!(line(&mut session), "USER em 0 * :Emily Example");
    assert_eq!(session.poll_transmit(), None);
}

#[test]
fn welcome_adopts_the_server_nick_and_cancels_the_timeout() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("oldnick"));
    session.handle_connected(now);

    session.handle_line(
        now + Duration::from_secs(1),
        b":irc.example 001 account :Welcome to the network",
    );

    assert_eq!(session.nick(), "account");
    assert_eq!(session.poll_timeout(), Some(now + Duration::from_secs(11)));
}

#[test]
fn registration_timeout_is_the_first_deadline_and_closes_once() {
    let now = Instant::now();
    let timeout = Duration::from_secs(23);
    let mut session = Session::new(Config::default().registration_timeout(timeout));
    session.handle_connected(now);
    assert_eq!(session.poll_timeout(), Some(now + timeout));

    session.handle_timeout(now + timeout.checked_sub(Duration::from_nanos(1)).unwrap());
    assert_eq!(session.poll_event(), None);
    assert_eq!(session.poll_timeout(), Some(now + timeout));

    session.handle_timeout(now + timeout);
    assert!(matches!(
        session.poll_event(),
        Some(Event::Close { reason }) if reason.contains("before 001")
    ));
    assert_eq!(session.poll_timeout(), None);
    session.handle_timeout(now + timeout);
    assert_eq!(session.poll_event(), None);
}

#[test]
fn tries_alternative_nicks_then_appends_bounded_underscores() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("emily")
            .alt_nicks(["emily-away", "emily-work"]),
    );
    session.handle_connected(now);
    for _ in 0..3 {
        session.poll_transmit();
    }

    session.handle_line(now, b":ergo.test 433 * emily :Nickname is already in use");
    assert_eq!(line(&mut session), "NICK emily-away");
    session.handle_line(
        now,
        b":ergo.test 437 * emily-away :Nick/channel is temporarily unavailable",
    );
    assert_eq!(line(&mut session), "NICK emily-work");
    session.handle_line(now, b":ergo.test 432 * emily-work :Erroneous nickname");
    assert_eq!(line(&mut session), "NICK emily-work_");
    session.handle_line(now, b":ergo.test 436 * emily-work_ :Nickname collision");
    assert_eq!(line(&mut session), "NICK emily-work__");

    session.handle_line(now, b":ergo.test 001 emily-work__ :Welcome");
    session.handle_line(
        now,
        b":ergo.test 433 emily-work__ someone :Nickname is already in use",
    );
    assert!(matches!(
        session.poll_event(),
        Some(Event::Message(message)) if message.as_ref().command_is("433")
    ));
}

#[test]
fn generated_nicks_stay_within_the_default_30_byte_limit() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("a-realistically-long-client-nick"));
    session.handle_connected(now);
    for _ in 0..3 {
        session.poll_transmit();
    }

    session.handle_line(
        now,
        b":solanum.example 433 * nick :Nickname is already in use",
    );
    let first = line(&mut session);
    assert_eq!(first.len(), "NICK ".len() + 30);
    assert!(first.ends_with('_'));
    session.handle_line(
        now,
        b":solanum.example 433 * nick :Nickname is already in use",
    );
    let second = line(&mut session);
    assert_eq!(second.len(), "NICK ".len() + 30);
    assert!(second.ends_with("__"));
    assert_ne!(first, second);
}

#[test]
fn generated_nicks_honor_an_early_nicklen_advertisement() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("long-client-nickname"));
    session.handle_connected(now);
    for _ in 0..3 {
        session.poll_transmit();
    }
    session.handle_line(
        now,
        b":ergo.test 005 * NICKLEN=12 :are supported by this server",
    );
    session.handle_line(now, b":ergo.test 433 * nick :Nickname is already in use");

    assert_eq!(line(&mut session), "NICK long-client_");
}

#[test]
fn completes_after_motd_with_capabilities_and_isupport() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("requested")
            .wanted_caps(["server-time"]),
    );
    session.handle_connected(now);
    for _ in 0..3 {
        session.poll_transmit();
    }
    session.handle_line(now, b"CAP * LS :server-time=iso8601");
    assert_eq!(line(&mut session), "CAP REQ :server-time");
    session.handle_line(now, b"CAP * ACK :server-time");
    assert_eq!(line(&mut session), "CAP END");
    session.handle_line(now, b":ergo.test 001 account :Welcome to Ergo");
    session.handle_line(
        now,
        b":ergo.test 005 account CASEMAPPING=ascii NICKLEN=32 :are supported by this server",
    );
    session.handle_line(
        now,
        b":ergo.test 005 account NETWORK=Ergo UTF8ONLY :are supported by this server",
    );
    session.handle_line(now, b":ergo.test 376 account :End of MOTD");

    assert!(session.is_registered());
    assert!(matches!(
        session.poll_event(),
        Some(Event::Registered(welcome))
            if welcome.nick == "account"
                && welcome.caps.get("server-time") == Some(&Some("iso8601".to_owned()))
                && welcome.isupport.network() == Some("Ergo")
                && welcome.isupport.utf8only()
    ));
}

#[test]
fn no_motd_numeric_completes_registration() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_connected(now);
    for _ in 0..3 {
        session.poll_transmit();
    }
    session.handle_line(now, b":solanum.example 001 papo :Welcome");
    session.handle_line(now, b":solanum.example 422 papo :MOTD File is missing");

    assert!(matches!(
        session.poll_event(),
        Some(Event::Registered(welcome)) if welcome.nick == "papo" && welcome.caps.is_empty()
    ));
    assert!(session.is_registered());
}

#[test]
fn completes_ten_seconds_after_welcome_when_motd_never_finishes() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_connected(now);
    session.handle_line(now, b":soju.example 001 papo :Welcome");
    assert_eq!(session.poll_timeout(), Some(now + Duration::from_secs(10)));

    session.handle_timeout(now + Duration::from_secs(9));
    assert!(!session.is_registered());
    assert_eq!(session.poll_event(), None);
    session.handle_timeout(now + Duration::from_secs(10));
    assert!(session.is_registered());
    assert!(matches!(session.poll_event(), Some(Event::Registered(_))));
    assert_eq!(session.poll_timeout(), None);
}

#[test]
fn reconnect_resets_all_registration_state() {
    Transcript::new(
        Config::default()
            .alt_nicks(["papo-away"])
            .wanted_caps(["server-time"]),
    )
    .connect_default()
    .server("CAP * LS :server-time")
    .client("CAP REQ :server-time")
    .server("CAP * ACK :server-time")
    .client("CAP END")
    .server(":ergo.test 433 * papo :Nickname is already in use")
    .client("NICK papo-away")
    .server(":ergo.test 005 * NICKLEN=32 NETWORK=ErgoTest :are supported by this server")
    .server(":ergo.test 001 papo-away :Welcome to Ergo")
    .server(":ergo.test 422 papo-away :MOTD File is missing")
    .event(|event| matches!(event, Event::Registered(_)))
    .inspect(|session| {
        assert!(session.is_registered());
        assert!(session.caps().contains_key("server-time"));
        assert_eq!(session.isupport().network(), Some("ErgoTest"));
        assert_eq!(session.nick(), "papo-away");
    })
    .disconnect()
    .inspect(|session| {
        assert!(!session.is_registered());
        assert_eq!(session.poll_timeout(), None);
    })
    .connect_default()
    .inspect(|session| {
        assert!(session.caps().is_empty());
        assert_eq!(session.isupport(), &Isupport::default());
        assert_eq!(session.nick(), "papo");
        assert!(session.poll_timeout().is_some());
    });
}
