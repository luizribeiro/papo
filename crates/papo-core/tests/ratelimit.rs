//! Transcript tests for outbound rate limiting.

mod support;

use std::time::Duration;

use papo_core::{Config, RateLimit};
use papo_proto::Line;
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
        transcript = transcript.send(Line::privmsg("#papo", format!("message {index}")));
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
fn default_limiter_honors_both_saferate_spellings() {
    for token in ["SAFERATE", "soju.im/SAFERATE"] {
        let mut transcript = Transcript::rate_limited(Config::default()).server(&format!(
            ":ergo.test 005 papo {token} :are supported by this server"
        ));
        for index in 1..=8 {
            transcript = transcript.send(Line::privmsg("#papo", format!("message {index}")));
        }
        for index in 1..=8 {
            transcript = transcript.client(&format!("PRIVMSG #papo :message {index}"));
        }
        transcript
            .no_client()
            .inspect(|session| assert_eq!(session.queued_len(), 0));
    }
}

#[test]
fn strict_limiter_ignores_saferate() {
    let mut transcript =
        Transcript::rate_limited(Config::default().rate_limit(RateLimit::default().strict()))
            .server(":ergo.test 005 papo SAFERATE :are supported by this server");
    for index in 1..=6 {
        transcript = transcript.send(Line::privmsg("#papo", format!("message {index}")));
    }
    for index in 1..=5 {
        transcript = transcript.client(&format!("PRIVMSG #papo :message {index}"));
    }
    transcript
        .no_client()
        .inspect(|session| assert_eq!(session.queued_len(), 1));
}

#[test]
fn one_interval_refills_exactly_one_token() {
    let mut transcript = Transcript::rate_limited(Config::default());
    for index in 1..=8 {
        transcript = transcript.send(Line::privmsg("#papo", format!("message {index}")));
    }
    for index in 1..=5 {
        transcript = transcript.client(&format!("PRIVMSG #papo :message {index}"));
    }
    transcript
        .advance(Duration::from_millis(500))
        .client("PRIVMSG #papo :message 6")
        .no_client()
        .advance(Duration::from_millis(500))
        .client("PRIVMSG #papo :message 7")
        .no_client()
        .advance(Duration::from_millis(500))
        .client("PRIVMSG #papo :message 8")
        .no_client();
}

#[test]
fn unconsumed_intervals_refill_the_whole_bucket() {
    let mut transcript = Transcript::rate_limited(Config::default());
    for index in 1..=10 {
        transcript = transcript.send(Line::privmsg("#papo", format!("message {index}")));
    }
    for index in 1..=5 {
        transcript = transcript.client(&format!("PRIVMSG #papo :message {index}"));
    }
    transcript = transcript.advance(Duration::from_millis(2_500));
    for index in 6..=10 {
        transcript = transcript.client(&format!("PRIVMSG #papo :message {index}"));
    }
    transcript.no_client();
}

#[test]
fn a_full_bucket_has_no_refill_deadline() {
    let mut transcript = Transcript::rate_limited(Config::default());
    for index in 1..=10 {
        transcript = transcript.send(Line::privmsg("#papo", format!("message {index}")));
    }
    for index in 1..=5 {
        transcript = transcript.client(&format!("PRIVMSG #papo :message {index}"));
    }
    transcript
        .advance(Duration::from_millis(2_500))
        .inspect(|session| assert_eq!(session.queued_len(), 5))
        .deadline(None)
        .ignore_rest();
}

#[test]
fn reconnecting_restores_the_full_burst() {
    let mut transcript = Transcript::rate_limited(Config::default()).connect_default();
    for index in 1..=5 {
        transcript = transcript.send(Line::privmsg("#papo", format!("before {index}")));
    }
    for index in 1..=5 {
        transcript = transcript.client(&format!("PRIVMSG #papo :before {index}"));
    }
    transcript = transcript.disconnect().connect_default();
    for index in 1..=5 {
        transcript = transcript.send(Line::privmsg("#papo", format!("after {index}")));
    }
    for index in 1..=5 {
        transcript = transcript.client(&format!("PRIVMSG #papo :after {index}"));
    }
    transcript.no_client();
}

#[test]
fn taking_another_token_preserves_the_refill_deadline() {
    Transcript::rate_limited(Config::default())
        .send(Line::privmsg("#papo", "first"))
        .send(Line::privmsg("#papo", "second"))
        .send(Line::privmsg("#papo", "third"))
        .client("PRIVMSG #papo first")
        .deadline(Some(Duration::from_millis(500)))
        .advance(Duration::from_millis(100))
        .client("PRIVMSG #papo second")
        .deadline(Some(Duration::from_millis(400)))
        .ignore_rest();
}

#[test]
fn taking_from_a_refilled_bucket_starts_a_new_interval() {
    let mut transcript = Transcript::rate_limited(Config::default());
    for index in 1..=10 {
        transcript = transcript.send(Line::privmsg("#papo", format!("message {index}")));
    }
    for index in 1..=5 {
        transcript = transcript.client(&format!("PRIVMSG #papo :message {index}"));
    }
    transcript
        .advance(Duration::from_millis(2_500))
        .client("PRIVMSG #papo :message 6")
        .deadline(Some(Duration::from_millis(500)))
        .ignore_rest();
}

#[test]
fn unlimited_rate_limit_never_schedules_a_token_deadline() {
    Transcript::rate_limited(Config::default().rate_limit(RateLimit::unlimited()))
        .send(Line::privmsg("#papo", "one"))
        .client("PRIVMSG #papo one")
        .deadline(None);
}

#[test]
fn queued_len_sums_regular_and_bypass_lines() {
    Transcript::rate_limited(Config::default())
        .send(Line::privmsg("#papo", "first"))
        .send(Line::privmsg("#papo", "second"))
        .send(Line::privmsg("#papo", "third"))
        .send(Line::new("PONG").with_params(["ergo.test-4917"]))
        .send(Line::new("CAP").with_params(["END"]))
        .inspect(|session| assert_eq!(session.queued_len(), 5))
        .ignore_rest();
}

#[test]
fn protocol_critical_commands_bypass_an_exhausted_bucket() {
    let mut transcript = Transcript::rate_limited(Config::default());
    for index in 1..=5 {
        transcript = transcript.send(Line::privmsg("#papo", format!("message {index}")));
    }
    for index in 1..=5 {
        transcript = transcript.client(&format!("PRIVMSG #papo :message {index}"));
    }
    for (message, expected) in [
        (Line::new("CAP").with_params(["END"]), "CAP END"),
        (
            Line::new("AUTHENTICATE").with_params(["+"]),
            "AUTHENTICATE +",
        ),
        (
            Line::new("PONG").with_params(["ergo.test"]),
            "PONG ergo.test",
        ),
    ] {
        transcript = transcript.send(message).client(expected);
    }
}

#[test]
fn quit_keeps_its_place_behind_queued_messages() {
    let mut transcript = Transcript::rate_limited(Config::default());
    for index in 1..=5 {
        transcript = transcript.send(Line::privmsg("#papo", format!("drain {index}")));
    }
    for index in 1..=5 {
        transcript = transcript.client(&format!("PRIVMSG #papo :drain {index}"));
    }
    transcript
        .send(Line::privmsg("#papo", "first"))
        .send(Line::privmsg("#papo", "second"))
        .send(Line::privmsg("#papo", "third"))
        .send(Line::new("QUIT").with_params(["shutting down"]))
        .no_client()
        .advance(Duration::from_millis(500))
        .client("PRIVMSG #papo first")
        .advance(Duration::from_millis(500))
        .client("PRIVMSG #papo second")
        .advance(Duration::from_millis(500))
        .client("PRIVMSG #papo third")
        .advance(Duration::from_millis(500))
        .client("QUIT :shutting down")
        .no_client();
}
