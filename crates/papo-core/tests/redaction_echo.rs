//! Transcript coverage for the confirmation of our own redactions.
//!
//! Ergo 2.19 answers a labelled `REDACT` with an `ACK` and sends the redaction's echo, which
//! carries its msgid, without the label and just before that `ACK`.

use std::time::Instant;

use papo_core::{Config, Event, Party, RateLimit, RequestId, Response, SentInfo, Session};
use papo_proto::Line;

const ERGO_ISUPPORT: &str = ":ergo.test 005 papo CASEMAPPING=ascii CHANTYPES=# PREFIX=(qaohv)~&@%+ STATUSMSG=~&@%+ :are supported by this server";
const REDACTION_CAPS: &str =
    "draft/message-redaction echo-message labeled-response message-tags server-time";
const REDACTED_MSGID: &str = "b56bhkz4gmj7ig2jeg9hs7d2cs";
const OWN_ECHO: &str = "@msgid=qsgqkhqcmnxm6x2dcricaf69f6;time=2026-11-16T07:39:12.791Z :papo!~u@fa4ww98eysn9a.irc REDACT #papo b56bhkz4gmj7ig2jeg9hs7d2cs superseded";
const ACK: &str = "@time=2026-11-16T07:39:12.791Z;label=0 :ergo.test ACK";

fn ergo(now: Instant, caps: &str) -> Session {
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    session.handle_connected(now);
    session.handle_line(now, format!(":ergo.test CAP * LS :{caps}").as_bytes());
    session.handle_line(now, format!(":ergo.test CAP * ACK :{caps}").as_bytes());
    for line in [
        ":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo",
        ERGO_ISUPPORT,
        ":ergo.test 422 papo :MOTD File is missing",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
    session
}

fn redact(session: &mut Session, now: Instant) -> RequestId {
    let id = session
        .request_silent(
            now,
            Line::new("REDACT").with_params(["#papo", REDACTED_MSGID, "superseded"]),
        )
        .unwrap();
    while session.poll_transmit().is_some() {}
    id
}

fn play(session: &mut Session, now: Instant, line: &str) -> Vec<Event> {
    session.handle_line(now, line.as_bytes());
    std::iter::from_fn(|| session.poll_event()).collect()
}

fn redaction_by(event: &Event) -> Option<Party> {
    let Event::Redacted(redacted) = event else {
        return None;
    };
    assert_eq!(redacted.msgid, REDACTED_MSGID);
    Some(redacted.actor_party)
}

fn response(event: &Event) -> &Response {
    let Event::Response(response) = event else {
        panic!("expected a response, got {event:?}");
    };
    response
}

fn own_echo_identity() -> SentInfo {
    SentInfo {
        msgid: "qsgqkhqcmnxm6x2dcricaf69f6".to_owned(),
        server_time: Some("2026-11-16T07:39:12.791Z".to_owned()),
    }
}

#[test]
fn ergos_echo_before_the_ack_confirms_the_redaction() {
    let now = Instant::now();
    let mut session = ergo(now, REDACTION_CAPS);
    let id = redact(&mut session, now);

    let echoed = play(&mut session, now, OWN_ECHO);
    assert_eq!(echoed.len(), 1, "{echoed:?}");
    assert_eq!(redaction_by(&echoed[0]), Some(Party::Me));

    let acked = play(&mut session, now, ACK);
    assert_eq!(acked.len(), 1, "{acked:?}");
    let response = response(&acked[0]);
    assert_eq!(response.id, id);
    assert_eq!(response.sent(), Some(own_echo_identity()));
    assert_eq!(response.error(), None);
}
