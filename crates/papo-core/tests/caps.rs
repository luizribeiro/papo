//! Transcript tests for `IRCv3` capability negotiation.

mod support;

use papo_core::{Config, Session};
use support::Transcript;

#[test]
fn waits_for_the_final_multiline_ls_reply() {
    Transcript::new(Config::default())
        .connect()
        .client("CAP LS 302")
        .client("NICK papo")
        .client("USER papo 0 * papo")
        .server("CAP * LS * :server-time=old sasl=PLAIN,EXTERNAL")
        .no_client()
        .server("CAP * LS :server-time=latest draft/no-implicit-names")
        .client("CAP END");

    let mut session = Session::new(Config::default());
    session.handle_connected(std::time::Instant::now());
    session.handle_line(std::time::Instant::now(), b"CAP * LS :");
    assert!(session.caps().contains_key("cap-notify"));
}

#[test]
fn ends_immediately_when_no_advertised_cap_is_wanted() {
    Transcript::new(Config::default().wanted_caps(Vec::<String>::new()))
        .connect_default()
        .server("CAP * LS :")
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
