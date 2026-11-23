//! Transcript tests for checking a doubtful connection.

mod support;

use std::time::Duration;

use papo_core::{Config, ConnectionCheckResult, Event};
use support::Transcript;

fn register(transcript: Transcript) -> Transcript {
    transcript
        .connect_default()
        .server(":ergo.test CAP * LS :")
        .client("CAP END")
        .welcome(":ergo.test 001 papo :Welcome to the Ergo IRC network papo")
        .server(":ergo.test 422 papo :MOTD File is missing")
        .inspect(|session| assert!(session.is_connected()))
        .event(|event| matches!(event, Event::Connected(_)))
}

#[test]
fn a_written_idle_ping_can_answer_a_later_check() {
    register(Transcript::new(Config::default()))
        .advance(Duration::from_secs(60))
        .client("PING papo-0")
        .advance(Duration::from_secs(1))
        .check_connection(Duration::from_secs(3), true)
        .no_client()
        .advance(Duration::from_secs(2))
        .server(":ergo.test PONG ergo.test :papo-0")
        .check_result(Some(ConnectionCheckResult::Pong(Duration::from_secs(3))));
}

#[test]
fn a_silent_connection_closes_at_the_explicit_deadline() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test CAP * LS :")
        .client("CAP END")
        .welcome(":ergo.test 001 papo :Welcome to the Ergo IRC network papo")
        .server(":ergo.test 422 papo :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
        .check_connection(Duration::from_secs(3), true)
        .advance(Duration::from_secs(3))
        .event(|event| matches!(event, Event::Close { .. }));
}
