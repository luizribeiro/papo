use std::time::Instant;

use papo_proto::Line;

use crate::{Config, Event, RateLimit, Refusal, Session};

fn labelled_session(now: Instant) -> Session {
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :labeled-response");
    session.handle_line(now, b"CAP * ACK :labeled-response");
    while session.poll_transmit().is_some() {}
    session
}

#[test]
fn config_does_not_select_a_bouncer_network() {
    assert_eq!(Config::default().selected_bouncer_network(), None);
}

#[test]
fn classifies_standard_and_numeric_refusals() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    session
        .request(
            now,
            Line::new("CHATHISTORY").with_params(["LATEST", "#papo", "bad", "50"]),
        )
        .unwrap();
    session
        .request(now, Line::new("JOIN").with_params(["#missing"]))
        .unwrap();
    session.handle_line(now, b"@label=0 :ergo.test BATCH +failure labeled-response");
    session.handle_line(
        now,
        b"@batch=failure :ergo.test FAIL CHATHISTORY INVALID_PARAMS #papo bad :Invalid message reference",
    );
    session.handle_line(now, b":ergo.test BATCH -failure");
    session.handle_line(
        now,
        b"@label=1 :solanum.example 403 papo #missing :No such channel",
    );

    let Some(Event::Response(fail)) = session.poll_event() else {
        panic!("expected standard failure response")
    };
    assert_eq!(fail.refusal(), Some(Refusal::InvalidParameters));
    let Some(Event::Response(numeric)) = session.poll_event() else {
        panic!("expected numeric failure response")
    };
    assert_eq!(numeric.refusal(), Some(Refusal::NoSuchChannel));
}
