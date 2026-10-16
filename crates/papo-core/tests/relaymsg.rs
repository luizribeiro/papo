//! Transcript tests for `draft/relaymsg`.

mod support;

use papo_core::{Config, Event, RequestError};
use support::Transcript;

fn registered(value: Option<&str>) -> Transcript {
    let advertised = value.map_or_else(
        || "draft/relaymsg".to_owned(),
        |value| format!("draft/relaymsg={value}"),
    );
    Transcript::new(Config::default().wanted_caps(["draft/relaymsg", "labeled-response"]))
        .connect_default()
        .server(&format!("CAP * LS :{advertised} labeled-response"))
        .client("CAP REQ :draft/relaymsg labeled-response")
        .server("CAP * ACK :draft/relaymsg labeled-response")
        .client("CAP END")
        .server(":ergo.test 001 bridge :Welcome to Ergo")
        .server(":ergo.test 422 bridge :MOTD File is missing")
        .event(|event| matches!(event, Event::Registered(_)))
}

#[test]
fn sends_a_relayed_message_with_an_advertised_separator() {
    registered(Some("/|"))
        .update(|session| {
            session
                .relay(
                    std::time::Instant::now(),
                    "#bridged",
                    "matrix|alice",
                    "Hello from Matrix",
                )
                .unwrap();
        })
        .client("@label=0 RELAYMSG #bridged matrix|alice :Hello from Matrix");
}

#[test]
fn validates_relayed_nicknames_against_the_capability_value() {
    for (value, nick, expected) in [
        (
            Some("/|"),
            "matrix-alice",
            RequestError::InvalidRelayNickname,
        ),
        (None, "matrix-alice", RequestError::InvalidRelayNickname),
    ] {
        registered(value)
            .update(|session| {
                assert_eq!(
                    session.relay(std::time::Instant::now(), "#bridged", nick, "hello"),
                    Err(expected)
                );
            })
            .no_client();
    }
    registered(None)
        .update(|session| {
            session
                .relay(
                    std::time::Instant::now(),
                    "#bridged",
                    "matrix/alice",
                    "hello",
                )
                .unwrap();
        })
        .client("@label=0 RELAYMSG #bridged matrix/alice hello");
}

#[test]
fn requires_the_relaymsg_capability() {
    Transcript::new(Config::default())
        .update(|session| {
            assert_eq!(
                session.relay(
                    std::time::Instant::now(),
                    "#bridged",
                    "matrix/alice",
                    "hello"
                ),
                Err(RequestError::Unsupported("relaymsg"))
            );
        })
        .no_client();
}
