//! Transcript tests for `IRCv3` capability negotiation.

mod support;

use papo_core::{Config, Event, Session};
use support::Transcript;

#[test]
fn collects_values_selects_aliases_and_denies_the_trap() {
    let config =
        Config::default().wanted_caps(["server-time", "no-implicit-names", "ergo.chat/nope"]);
    Transcript::new(config)
        .connect()
        .client("CAP LS 302")
        .client("NICK papo")
        .client("USER papo 0 * papo")
        .server("CAP * LS * :server-time=old sasl=PLAIN,EXTERNAL")
        .no_client()
        .server("CAP * LS :server-time=latest draft/no-implicit-names")
        .client("CAP REQ :server-time draft/no-implicit-names")
        .no_client()
        .server("CAP * ACK :server-time draft/no-implicit-names")
        .client("CAP END");

    let mut session =
        Session::new(Config::default().wanted_caps(["server-time", "no-implicit-names"]));
    session.handle_connected(std::time::Instant::now());
    session.handle_line(
        std::time::Instant::now(),
        b"CAP * LS :server-time=latest draft/no-implicit-names",
    );
    session.handle_line(
        std::time::Instant::now(),
        b"CAP * ACK :server-time draft/no-implicit-names",
    );
    assert_eq!(
        session.caps().get("server-time"),
        Some(&Some("latest".to_owned()))
    );
    assert_eq!(session.caps().get("draft/no-implicit-names"), Some(&None));
    assert!(session.caps().contains_key("cap-notify"));
}

#[test]
fn ends_immediately_when_no_advertised_cap_is_wanted() {
    Transcript::new(Config::default().wanted_caps(Vec::<String>::new()))
        .connect_default()
        .server("CAP * LS :")
        .client("CAP END");

    Transcript::new(Config::default().wanted_caps(["ergo.chat/nope"]))
        .connect_default()
        .server("CAP * LS :ergo.chat/nope")
        .client("CAP END");
}

#[test]
fn drops_a_rejected_request_and_ends_negotiation() {
    Transcript::new(Config::default().wanted_caps(["batch", "server-time"]))
        .connect_default()
        .server("CAP * LS :batch server-time")
        .client("CAP REQ :batch server-time")
        .server("CAP * NAK :batch server-time")
        .client("CAP END");
}

#[test]
fn splits_requests_at_510_bytes_and_resolves_each_atomically() {
    let first = format!("example.com/{}", "a".repeat(238));
    let second = format!("example.net/{}", "b".repeat(238));
    let third = "batch".to_owned();
    let config = Config::default().wanted_caps([first.clone(), second.clone(), third.clone()]);
    Transcript::new(config)
        .connect_default()
        .server(&format!("CAP * LS * :{first}"))
        .server(&format!("CAP * LS * :{second}"))
        .server(&format!("CAP * LS :{third}"))
        .client(&format!("CAP REQ :{first} {second}"))
        .client("CAP REQ :batch")
        .server("CAP * NAK :request rejected")
        .no_client()
        .server("CAP * ACK * :bat")
        .no_client()
        .server("CAP * ACK :ch")
        .client("CAP END")
        .no_client();

    let first_request = format!("CAP REQ :{first} {second}");
    assert_eq!(first_request.len(), 510);
}

#[test]
fn keeps_510_bytes_together_and_splits_511_bytes() {
    let first = format!("example.com/{}", "a".repeat(238));
    let fits = format!("example.net/{}", "b".repeat(238));
    let exact = format!("CAP REQ :{first} {fits}");
    assert_eq!(exact.len(), 510);
    Transcript::new(Config::default().wanted_caps([&first, &fits]))
        .connect_default()
        .server(&format!("CAP * LS * :{first}"))
        .server(&format!("CAP * LS :{fits}"))
        .client(&exact)
        .no_client()
        .server("CAP * ACK :accepted")
        .client("CAP END");

    let splits = format!("example.net/{}", "b".repeat(239));
    let too_long = format!("CAP REQ :{first} {splits}");
    assert_eq!(too_long.len(), 511);
    Transcript::new(Config::default().wanted_caps([&first, &splits]))
        .connect_default()
        .server(&format!("CAP * LS * :{first}"))
        .server(&format!("CAP * LS :{splits}"))
        .client(&format!("CAP REQ :{first}"))
        .client(&format!("CAP REQ :{splits}"))
        .server("CAP * ACK :first accepted")
        .no_client()
        .server("CAP * ACK :second accepted")
        .client("CAP END");
}

#[test]
fn ignores_cap_errors_and_waits_for_welcome() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":legacy.example 421 papo CAP :Unknown command")
        .no_client()
        .server(":legacy.example 001 papo :Welcome")
        .no_client();

    Transcript::new(Config::default())
        .connect_default()
        .server(":broken.example 410 papo LS :Invalid CAP command")
        .no_client()
        .server(":broken.example 001 papo :Welcome")
        .no_client();
}

#[test]
fn surfaces_unknown_command_for_commands_other_than_cap() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":legacy.example 421 papo WHOIS :Unknown command")
        .event(|event| {
            matches!(
                event,
                Event::Message(message)
                    if message.as_ref().command_is("421")
                        && message.as_ref().param(1) == Some("WHOIS")
            )
        });
}
