//! Transcript tests for server liveness messages.

mod support;

use papo_core::{Config, Event};
use support::Transcript;

#[test]
fn answers_pings_before_and_after_server_data() {
    Transcript::new(Config::default())
        .connect()
        .server(":ergo.test PING :before-registration")
        .client("PONG :before-registration")
        .server(":ergo.test NOTICE tester :Connection established")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().command_is("NOTICE")))
        .server("PING :after-data")
        .client("PONG :after-data")
        .no_client();
}

#[test]
fn closes_when_the_server_sends_error() {
    Transcript::new(Config::default())
        .connect()
        .server("ERROR :Closing link")
        .event(|event| matches!(event, Event::Close { reason } if reason == "Closing link"))
        .server("PING :ignored-after-error")
        .no_client();
}

#[test]
fn closes_on_a_parameterless_error() {
    Transcript::new(Config::default())
        .connect()
        .server("ERROR")
        .event(|event| matches!(event, Event::Close { reason } if reason == "server sent ERROR"))
        .no_client();
}

#[test]
fn does_not_answer_invalid_pings() {
    Transcript::new(Config::default())
        .connect()
        .server(":ergo.test PING")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().command_is("PING")))
        .server("PING :bad\rtoken")
        .event(|event| matches!(event, Event::Malformed { line } if line.as_slice() == b"PING :bad\rtoken"))
        .no_client();
}
