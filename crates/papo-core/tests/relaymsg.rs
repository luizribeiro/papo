//! Transcript tests for `draft/relaymsg`.
#![cfg(feature = "unstable")]

mod support;

use papo_core::{Config, Event, Feature, Refusal, RequestError};
use support::{Transcript, unsupported_because};

fn registered(value: Option<&str>) -> Transcript {
    let advertised = value.map_or_else(
        || "draft/relaymsg".to_owned(),
        |value| format!("draft/relaymsg={value}"),
    );
    Transcript::new(
        Config::default()
            .enable(Feature::Relay)
            .wanted_caps(["draft/relaymsg", "labeled-response"]),
    )
    .connect_default()
    .server(&format!("CAP * LS :{advertised} labeled-response"))
    .client("CAP REQ :draft/relaymsg labeled-response")
    .server("CAP * ACK :draft/relaymsg labeled-response")
    .client("CAP END")
    .welcome(":ergo.test 001 bridge :Welcome to Ergo")
    .server(":ergo.test 422 bridge :MOTD File is missing")
    .event(|event| matches!(event, Event::Connected(_)))
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
fn rejects_multiple_relay_targets_without_queueing() {
    registered(Some("/"))
        .update(|session| {
            assert_eq!(
                session.relay(
                    std::time::Instant::now(),
                    "#bridged,alice",
                    "matrix/alice",
                    "private bridge message",
                ),
                Err(RequestError::Build(papo_proto::BuildError::InvalidParam(0)))
            );
        })
        .no_client();
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
                Err(unsupported_because(
                    Feature::Relay,
                    papo_core::UnsupportedReason::DisabledByClient,
                ))
            );
        })
        .no_client();
}

#[test]
fn types_every_relaymsg_failure() {
    for (code, expected) in [
        ("NOT_ENABLED", Refusal::RelayDisabled),
        ("PRIVS_NEEDED", Refusal::RelayPrivilegesNeeded),
        ("BLANK_MSG", Refusal::BlankRelayMessage),
        ("INVALID_NICK", Refusal::InvalidRelayNickname),
        ("BANNED", Refusal::BannedRelaySender),
    ] {
        registered(Some("/"))
            .server(&format!(
                ":ergo.test FAIL RELAYMSG {code} #bridged :Relay request rejected"
            ))
            .event(|event| {
                matches!(event, Event::StandardReply(reply)
                if reply.refusal() == Some(expected))
            });
    }
    registered(Some("/"))
        .server(":ergo.test FAIL PRIVMSG BANNED #bridged :Message rejected")
        .event(|event| {
            matches!(event, Event::StandardReply(reply)
            if reply.refusal() == Some(Refusal::Other))
        });
}

#[test]
fn types_a_labelled_relaymsg_failure() {
    registered(Some("/"))
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
        .client("@label=0 RELAYMSG #bridged matrix/alice hello")
        .server("@label=0 :ergo.test FAIL RELAYMSG BANNED #bridged :Relayed sender is banned")
        .event(|event| {
            matches!(event, Event::Response(response)
            if response.refusal() == Some(Refusal::BannedRelaySender))
        });
}
