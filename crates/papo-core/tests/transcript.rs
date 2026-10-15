//! Tests for the shared transcript driver.

mod support;

use std::time::Duration;
#[cfg(panic = "unwind")]
use std::{any::Any, panic};

use papo_core::{Config, Event};
use papo_proto::Message;
use support::Transcript;

#[cfg(panic = "unwind")]
fn panic_text(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(ToString::to_string))
        .expect("panic contains text")
}

#[test]
fn drives_server_lines_events_and_time() {
    Transcript::new(Config::default())
        .connect()
        .send(Message::new("NICK", ["tester"]))
        .client("NICK tester")
        .server(":ergo.test NOTICE tester :Connection established")
        .no_client()
        .advance(Duration::from_secs(30))
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().command_is("NOTICE")));
}

#[test]
fn plays_trace_file_contents() {
    Transcript::new(Config::default())
        .connect()
        .send(Message::new("NICK", ["tester"]))
        .play(include_str!("data/harness.irc"))
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().command_is("NOTICE")))
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().command_is("JOIN")))
        .no_client();
}

#[test]
#[cfg(not(all(target_os = "wasi", not(target_env = "p1"))))]
fn loads_trace_files() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/harness.irc");
    Transcript::new(Config::default())
        .send(Message::new("NICK", ["tester"]))
        .load(path)
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().command_is("NOTICE")));
}

#[test]
#[cfg(panic = "unwind")]
fn reports_unexpected_client_output_with_a_readable_diff() {
    let panic = panic::catch_unwind(|| {
        Transcript::new(Config::default())
            .send(Message::new("NICK", ["tester"]))
            .client("NICK someone-else");
    })
    .expect_err("mismatched output must fail");
    let text = panic_text(panic.as_ref());
    assert!(text.contains("expected next:\n  C: NICK someone-else"));
    assert!(text.contains("actual next:\n  C: NICK tester"));
    assert!(text.contains("client lines sent so far:\n  <none>"));
}

#[test]
#[cfg(panic = "unwind")]
fn reports_a_missing_expected_client_line() {
    let panic = panic::catch_unwind(|| {
        Transcript::new(Config::default()).client("CAP LS 302");
    })
    .expect_err("missing output must fail");
    let text = panic_text(panic.as_ref());
    assert!(text.contains("expected next:\n  C: CAP LS 302"));
    assert!(text.contains("actual next:\n  <nothing>"));
}
