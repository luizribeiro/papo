//! Transcript tests for `IRCv3` capability negotiation.

mod support;

use papo_core::{Config, Session};
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
fn never_emits_a_request_over_510_bytes() {
    let first = format!("example.com/{}", "a".repeat(239));
    let second = format!("example.net/{}", "b".repeat(239));
    let now = std::time::Instant::now();
    let mut session = Session::new(Config::default().wanted_caps([&first, &second]));
    session.handle_connected(now);
    for _ in 0..3 {
        session.poll_transmit();
    }
    session.handle_line(now, format!("CAP * LS * :{first}").as_bytes());
    session.handle_line(now, format!("CAP * LS :{second}").as_bytes());

    let requests = [
        session.poll_transmit().unwrap(),
        session.poll_transmit().unwrap(),
    ];
    assert!(requests.iter().all(|line| line.len() - 2 <= 510));
    assert_eq!(session.poll_transmit(), None);
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
