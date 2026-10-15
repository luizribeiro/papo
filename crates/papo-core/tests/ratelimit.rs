//! Transcript tests for outbound rate limiting.

mod support;

use std::time::Duration;

use papo_core::{Config, RateLimit};
use papo_proto::Message;
use support::Transcript;

#[test]
fn releases_five_lines_then_one_every_500_milliseconds() {
    let mut transcript = Transcript::rate_limited(Config::default());
    for index in 1..=8 {
        transcript = transcript.send(Message::privmsg("#papo", format!("message {index}")));
    }
    for index in 1..=5 {
        transcript = transcript.client(&format!("PRIVMSG #papo :message {index}"));
    }
    transcript
        .inspect(|session| assert_eq!(session.queued_len(), 3))
        .no_client()
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
        .inspect(|session| assert_eq!(session.queued_len(), 0))
        .deadline(None);
}

#[test]
fn unlimited_rate_limit_never_schedules_a_token_deadline() {
    Transcript::rate_limited(Config::default().rate_limit(RateLimit::unlimited()))
        .send(Message::privmsg("#papo", "one"))
        .client("PRIVMSG #papo one")
        .deadline(None);
}
