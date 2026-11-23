//! Transcript tests for checking a doubtful connection.

mod support;

use std::time::Duration;

use papo_core::{CloseKind, Config, ConnectionCheckResult, Event, RateLimit};
use papo_proto::Line;
use support::Transcript;

fn connected() -> Transcript {
    register(Transcript::new(Config::default()))
}

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
fn overlapping_checks_share_the_first_deadline_and_round_trip() {
    connected()
        .check_connection(Duration::from_secs(3), true)
        .client("PING papo-check-0")
        .advance(Duration::from_secs(1))
        .check_connection(Duration::from_millis(50), true)
        .deadline(Some(Duration::from_secs(2)))
        .no_client()
        .advance(Duration::from_secs(1))
        .server(":ergo.test PONG ergo.test :papo-check-0")
        .check_result(Some(ConnectionCheckResult::Pong(Duration::from_secs(2))));
}

#[test]
fn silence_closes_at_the_deadline_even_if_readable_late() {
    for late_pong in [false, true] {
        let transcript = connected()
            .check_connection(Duration::from_secs(3), true)
            .client("PING papo-check-0")
            .advance(Duration::from_millis(2999))
            .check_result(None);
        let transcript = if late_pong {
            transcript.server_after(
                Duration::from_millis(2),
                ":ergo.test PONG ergo.test :papo-check-0",
            )
        } else {
            transcript.advance(Duration::from_millis(1))
        };
        transcript
            .check_result(Some(ConnectionCheckResult::Silent))
            .event(|event| {
                matches!(
                    event,
                    Event::Close {
                        kind: CloseKind::PingTimeout,
                        ..
                    }
                )
            });
    }
}

#[test]
fn an_unrelated_line_proves_liveness_but_supplies_no_round_trip() {
    connected()
        .check_connection(Duration::from_secs(3), true)
        .client("PING papo-check-0")
        .advance(Duration::from_secs(2))
        .server(":ergo.test NOTICE papo :You are connected to Ergo")
        .event(|event| matches!(event, Event::Line(_)))
        .check_result(None)
        .advance(Duration::from_secs(1))
        .check_result(Some(ConnectionCheckResult::TimedOut))
        .server("PING :ergo.test")
        .client("PONG ergo.test");
}

#[test]
fn a_check_during_registration_leaves_registration_alone() {
    Transcript::new(Config::default())
        .connect_default()
        .check_connection(Duration::from_secs(3), false)
        .no_client()
        .advance(Duration::from_secs(3))
        .check_result(None);
}

#[test]
fn a_queued_idle_ping_stays_paced_behind_an_explicit_check() {
    let config = Config::default().rate_limit(RateLimit::new(1, Duration::from_secs(30)).unwrap());
    register(Transcript::rate_limited(config))
        .advance(Duration::from_secs(59))
        .send(Line::privmsg("#papo", "first update"))
        .client("PRIVMSG #papo :first update")
        .send(Line::privmsg("#papo", "second update"))
        .advance(Duration::from_secs(1))
        .no_client()
        .check_connection(Duration::from_secs(3), true)
        .client("PING papo-check-0")
        .no_client()
        .advance(Duration::from_secs(1))
        .server(":ergo.test PONG ergo.test :papo-check-0")
        .check_result(Some(ConnectionCheckResult::Pong(Duration::from_secs(1))))
        .advance(Duration::from_secs(28))
        .client("PRIVMSG #papo :second update")
        .advance(Duration::from_secs(30))
        .client("PING papo-0");
}

#[test]
fn a_check_supplies_the_deadline_while_idle_keepalive_is_due() {
    let config = Config::default()
        .ping_interval(Duration::from_secs(1))
        .ping_timeout(Duration::from_secs(2));
    register(Transcript::new(config))
        .check_connection(Duration::from_secs(3), true)
        .client("PING papo-check-0")
        .deadline(Some(Duration::from_secs(3)))
        .advance(Duration::from_millis(2500))
        .no_client()
        .no_event()
        .server(":ergo.test PONG ergo.test :papo-check-0")
        .check_result(Some(ConnectionCheckResult::Pong(Duration::from_millis(
            2500,
        ))));
}

#[test]
fn a_written_idle_ping_can_answer_a_later_check() {
    connected()
        .advance(Duration::from_secs(60))
        .client("PING papo-0")
        .advance(Duration::from_secs(1))
        .check_connection(Duration::from_secs(3), true)
        .no_client()
        .advance(Duration::from_secs(2))
        .server(":ergo.test PONG ergo.test :papo-0")
        .check_result(Some(ConnectionCheckResult::Pong(Duration::from_secs(3))));
}
