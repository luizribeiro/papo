//! Transcript tests for checking a doubtful connection.

mod support;

use std::time::Duration;

use papo_core::{Config, Event};
use support::Transcript;

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
