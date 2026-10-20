//! Transcript tests for server liveness messages.

mod support;

use std::time::Duration;

use papo_core::{CloseKind, Config, Event};
use support::Transcript;

#[test]
fn answers_pings_before_and_after_server_data() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test PING :before-registration")
        .client("PONG :before-registration")
        .server(":ergo.test NOTICE tester :Connection established")
        .event(|event| matches!(event, Event::Message(message) | Event::Line(message) if message.as_ref().command_is("NOTICE")))
        .server("PING :after-data")
        .client("PONG :after-data")
        .no_client();
}

#[test]
fn closes_when_the_server_sends_error() {
    Transcript::new(Config::default())
        .connect_default()
        .server("ERROR :Closing link")
        .event(|event| matches!(event, Event::Close { kind: CloseKind::ServerError, reason } if reason == "Closing link"))
        .server("PING :ignored-after-error")
        .no_client();
}

#[test]
fn closes_on_a_parameterless_error() {
    Transcript::new(Config::default())
        .connect_default()
        .server("ERROR")
        .event(|event| matches!(event, Event::Close { kind: CloseKind::ServerError, reason } if reason == "server sent ERROR"))
        .no_client();
}

#[test]
fn bounds_server_error_reasons_without_splitting_utf8() {
    let exact = format!("K-lined: {}", "open proxy detected; ".repeat(24));
    let exact = &exact[..256];
    Transcript::new(Config::default())
        .connect_default()
        .server(&format!("ERROR :{exact}"))
        .event(|event| matches!(event, Event::Close { kind: CloseKind::ServerError, reason } if reason == exact));

    for (over, expected) in [
        ("K".repeat(257), "K".repeat(256)),
        (format!("{}é", "K".repeat(255)), "K".repeat(255)),
        (format!("{}€", "K".repeat(254)), "K".repeat(254)),
        (format!("{}🦀", "K".repeat(253)), "K".repeat(253)),
    ] {
        assert_eq!(over.len(), 257);
        Transcript::new(Config::default())
            .connect_default()
            .server(&format!("ERROR :{over}"))
            .event(|event| matches!(event, Event::Close { kind: CloseKind::ServerError, reason } if reason == &expected));
    }
}

#[test]
fn does_not_answer_invalid_pings() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test PING")
        .event(|event| matches!(event, Event::Message(message) | Event::Line(message) if message.as_ref().command_is("PING")))
        .server("PING :bad\rtoken")
        .event(|event| matches!(event, Event::Malformed { line } if line.as_slice() == b"PING :bad\rtoken"))
        .no_client();
}

#[test]
fn pings_after_sixty_idle_seconds_and_measures_matching_pongs() {
    Transcript::new(Config::default().registration_timeout(Duration::from_secs(300)))
        .connect_default()
        .deadline(Some(Duration::from_secs(60)))
        .advance(Duration::from_secs(59))
        .no_client()
        .server(":ergo.test")
        .event(|event| matches!(event, Event::Malformed { .. }))
        .advance(Duration::from_secs(59))
        .no_client()
        .advance(Duration::from_secs(1))
        .client("PING :papo-0")
        .server(":ergo.test PONG ergo.test :another-token")
        .inspect(|session| assert_eq!(session.lag(), None))
        .advance(Duration::from_millis(250))
        .server(":ergo.test PONG ergo.test :papo-0")
        .inspect(|session| assert_eq!(session.lag(), Some(Duration::from_millis(250))))
        .advance(Duration::from_secs(60))
        .client("PING :papo-1");
}

#[test]
fn closes_at_120_seconds_of_silence_but_any_pong_resets_the_timeout() {
    let config = Config::default().registration_timeout(Duration::from_secs(300));
    Transcript::new(config.clone())
        .connect_default()
        .advance(Duration::from_secs(60))
        .client("PING :papo-0")
        .advance(Duration::from_secs(59))
        .no_event()
        .advance(Duration::from_secs(1))
        .event(|event| matches!(event, Event::Close { kind: CloseKind::PingTimeout, reason } if reason == "IRC connection timed out"))
        .no_client();

    Transcript::new(config)
        .connect_default()
        .advance(Duration::from_secs(60))
        .client("PING :papo-0")
        .advance(Duration::from_secs(59))
        .server(":ergo.test PONG ergo.test :papo-0")
        .inspect(|session| assert_eq!(session.lag(), Some(Duration::from_secs(59))))
        .advance(Duration::from_secs(60))
        .client("PING :papo-1")
        .no_event();
}
