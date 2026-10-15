//! Transcript and deadline tests for connection registration.

mod support;

use std::time::Duration;

use papo_core::{Config, Event};
use papo_proto::Isupport;
use support::Transcript;

#[test]
fn sends_registration_in_protocol_order() {
    let config = Config::default()
        .nick("emily")
        .username("em")
        .realname("Emily Example")
        .password("correct horse battery staple");
    Transcript::new(config)
        .connect()
        .client("CAP LS 302")
        .client("PASS :correct horse battery staple")
        .client("NICK emily")
        .client("USER em 0 * :Emily Example")
        .no_client();
}

#[test]
fn welcome_adopts_the_server_nick_and_cancels_the_timeout() {
    Transcript::new(Config::default().nick("oldnick"))
        .connect()
        .client("CAP LS 302")
        .client("NICK oldnick")
        .client("USER papo 0 * papo")
        .advance(Duration::from_secs(1))
        .server(":irc.example 001 account :Welcome to the network")
        .inspect(|session| assert_eq!(session.nick(), "account"))
        .deadline(Some(Duration::from_secs(10)));
}

#[test]
fn registration_timeout_is_the_first_deadline_and_closes_once() {
    let timeout = Duration::from_secs(23);
    Transcript::new(Config::default().registration_timeout(timeout))
        .connect_default()
        .deadline(Some(timeout))
        .advance(timeout.checked_sub(Duration::from_nanos(1)).unwrap())
        .no_event()
        .deadline(Some(Duration::from_nanos(1)))
        .advance(Duration::from_nanos(1))
        .event(|event| matches!(event, Event::Close { reason } if reason.contains("before 001")))
        .deadline(None)
        .advance(Duration::ZERO)
        .no_event();
}

#[test]
fn tries_alternative_nicks_then_appends_bounded_underscores() {
    Transcript::new(
        Config::default()
            .nick("emily")
            .alt_nicks(["emily-away", "emily-work"]),
    )
    .connect()
    .client("CAP LS 302")
    .client("NICK emily")
    .client("USER papo 0 * papo")
    .server(":ergo.test 433 * emily :Nickname is already in use")
    .client("NICK emily-away")
    .server(":ergo.test 437 * emily-away :Nick/channel is temporarily unavailable")
    .client("NICK emily-work")
    .server(":ergo.test 432 * emily-work :Erroneous nickname")
    .client("NICK emily-work_")
    .server(":ergo.test 436 * emily-work_ :Nickname collision")
    .client("NICK emily-work__")
    .server(":ergo.test 001 emily-work__ :Welcome")
    .server(":ergo.test 433 emily-work__ someone :Nickname is already in use")
    .event(|event| matches!(event, Event::Message(message) if message.as_ref().command_is("433")));
}

#[test]
fn generated_nicks_stay_within_the_default_30_byte_limit() {
    let requested = "a-realistically-long-client-nick";
    let first = format!("NICK {}_", &requested[..29]);
    let second = format!("NICK {}__", &requested[..28]);
    assert_eq!(first.len(), "NICK ".len() + 30);
    assert_eq!(second.len(), "NICK ".len() + 30);
    assert_ne!(first, second);

    Transcript::new(Config::default().nick(requested))
        .connect()
        .client("CAP LS 302")
        .client(&format!("NICK {requested}"))
        .client("USER papo 0 * papo")
        .server(":solanum.example 433 * nick :Nickname is already in use")
        .client(&first)
        .server(":solanum.example 433 * nick :Nickname is already in use")
        .client(&second);
}

#[test]
fn generated_nicks_honor_an_early_nicklen_advertisement() {
    Transcript::new(Config::default().nick("long-client-nickname"))
        .connect()
        .client("CAP LS 302")
        .client("NICK long-client-nickname")
        .client("USER papo 0 * papo")
        .server(":ergo.test 005 * NICKLEN=12 :are supported by this server")
        .server(":ergo.test 433 * nick :Nickname is already in use")
        .client("NICK long-client_");
}

#[test]
fn completes_after_motd_with_capabilities_and_isupport() {
    Transcript::new(
        Config::default()
            .nick("requested")
            .wanted_caps(["server-time"]),
    )
    .connect()
    .client("CAP LS 302")
    .client("NICK requested")
    .client("USER papo 0 * papo")
    .server("CAP * LS :server-time=iso8601")
    .client("CAP REQ :server-time")
    .server("CAP * ACK :server-time")
    .client("CAP END")
    .server(":ergo.test 001 account :Welcome to Ergo")
    .server(":ergo.test 005 account CASEMAPPING=ascii NICKLEN=32 :are supported by this server")
    .server(":ergo.test 005 account NETWORK=Ergo UTF8ONLY :are supported by this server")
    .server(":ergo.test 376 account :End of MOTD")
    .inspect(|session| assert!(session.is_registered()))
    .event(|event| {
        matches!(
            event,
            Event::Registered(welcome)
                if welcome.nick == "account"
                    && welcome.caps.get("server-time") == Some(&Some("iso8601".to_owned()))
                    && welcome.isupport.network() == Some("Ergo")
                    && welcome.isupport.utf8only()
        )
    });
}

#[test]
fn no_motd_numeric_completes_registration() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":solanum.example 001 papo :Welcome")
        .server(":solanum.example 422 papo :MOTD File is missing")
        .event(|event| {
            matches!(
                event,
                Event::Registered(welcome)
                    if welcome.nick == "papo" && welcome.caps.is_empty()
            )
        })
        .inspect(|session| assert!(session.is_registered()));
}

#[test]
fn completes_ten_seconds_after_welcome_when_motd_never_finishes() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":soju.example 001 papo :Welcome")
        .deadline(Some(Duration::from_secs(10)))
        .advance(Duration::from_secs(9))
        .inspect(|session| assert!(!session.is_registered()))
        .no_event()
        .deadline(Some(Duration::from_secs(1)))
        .advance(Duration::from_secs(1))
        .inspect(|session| assert!(session.is_registered()))
        .event(|event| matches!(event, Event::Registered(_)))
        .deadline(None);
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
