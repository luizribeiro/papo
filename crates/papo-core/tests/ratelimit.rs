//! Transcript tests for outbound rate limiting.

mod support;

use std::time::Duration;

use papo_core::{Config, RateLimit};
use papo_proto::Message;
use support::Transcript;

#[test]
fn releases_five_lines_then_one_every_500_milliseconds() {
    let mut transcript = Transcript::rate_limited(
        Config::default()
            .ping_interval(Duration::from_secs(3_600))
            .ping_timeout(Duration::from_secs(7_200)),
    )
    .connect_default();
    for index in 1..=8 {
        transcript = transcript.send(Message::privmsg("#papo", format!("message {index}")));
    }
    for index in 1..=5 {
        transcript = transcript.client(&format!("PRIVMSG #papo :message {index}"));
    }
    transcript
        .inspect(|session| assert_eq!(session.queued_len(), 3))
        .no_client()
        .server("PING :ergo.test-4917")
        .client("PONG :ergo.test-4917")
        .deadline(Some(Duration::from_millis(500)))
        .advance(Duration::from_millis(499))
        .no_client()
        .advance(Duration::from_millis(1))
        .client("PRIVMSG #papo :message 6")
        .deadline(Some(Duration::from_millis(500)))
        .advance(Duration::from_millis(500))
        .client("PRIVMSG #papo :message 7")
        .advance(Duration::from_millis(500))
        .client("PRIVMSG #papo :message 8")
        .inspect(|session| assert_eq!(session.queued_len(), 0));
}

#[test]
fn one_interval_refills_exactly_one_token() {
    let mut transcript = Transcript::rate_limited(Config::default());
    for index in 1..=6 {
        transcript = transcript.send(Message::privmsg("#papo", format!("message {index}")));
    }
    for index in 1..=5 {
        transcript = transcript.client(&format!("PRIVMSG #papo :message {index}"));
    }
    transcript
        .advance(Duration::from_millis(500))
        .client("PRIVMSG #papo :message 6")
        .no_client();
}

#[test]
fn unlimited_rate_limit_never_schedules_a_token_deadline() {
    Transcript::rate_limited(Config::default().rate_limit(RateLimit::unlimited()))
        .send(Message::privmsg("#papo", "one"))
        .client("PRIVMSG #papo one")
        .deadline(None);
}

#[test]
fn protocol_critical_commands_bypass_an_exhausted_bucket() {
    let mut transcript = Transcript::rate_limited(Config::default());
    for index in 1..=5 {
        transcript = transcript.send(Message::privmsg("#papo", format!("message {index}")));
    }
    for index in 1..=5 {
        transcript = transcript.client(&format!("PRIVMSG #papo :message {index}"));
    }
    for (message, expected) in [
        (Message::new("CAP", ["END"]), "CAP END"),
        (Message::new("AUTHENTICATE", ["+"]), "AUTHENTICATE +"),
        (Message::new("PONG", ["ergo.test"]), "PONG ergo.test"),
        (
            Message::new("QUIT", ["shutting down"]),
            "QUIT :shutting down",
        ),
    ] {
        transcript = transcript.send(message).client(expected);
    }
}
