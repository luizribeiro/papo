//! Transcript coverage for the confirmation of our own redactions.
//!
//! Ergo 2.19 answers a labelled `REDACT` with an `ACK` and sends the redaction's echo, which
//! carries its msgid, without the label and just before that `ACK`. The `ACK` ends the request,
//! with that echo when it came first and without one otherwise.

use std::time::Instant;

use papo_core::{
    Config, Event, Party, RateLimit, RequestId, Response, SentInfo, ServerError, Session,
};
use papo_proto::Line;

const ERGO_ISUPPORT: &str = ":ergo.test 005 papo CASEMAPPING=ascii CHANTYPES=# PREFIX=(qaohv)~&@%+ STATUSMSG=~&@%+ :are supported by this server";
const REDACTION_CAPS: &str =
    "draft/message-redaction echo-message labeled-response message-tags server-time";
const REDACTED_MSGID: &str = "b56bhkz4gmj7ig2jeg9hs7d2cs";
const OWN_ECHO: &str = "@msgid=qsgqkhqcmnxm6x2dcricaf69f6;time=2026-11-16T07:39:12.791Z :papo!~u@fa4ww98eysn9a.irc REDACT #papo b56bhkz4gmj7ig2jeg9hs7d2cs superseded";
const ACK: &str = "@time=2026-11-16T07:39:12.791Z;label=0 :ergo.test ACK";
const FOREIGN_REDACT: &str = "@msgid=4f6fb68i4ak2gs3dzkqdf2rwa6;time=2026-11-16T07:39:16.699Z :bob!~u@fa4ww98eysn9a.irc REDACT #papo b56bhkz4gmj7ig2jeg9hs7d2cs :mod action";
const UNKNOWN_MSGID: &str = "@time=2026-11-16T07:39:19.107Z;label=0 :ergo.test FAIL REDACT UNKNOWN_MSGID #papo b56bhkz4gmj7ig2jeg9hs7d2cs :This message does not exist or is too old";

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
    assert_eq!(play(&mut session, now, OWN_ECHO).len(), 1);
}

#[test]
fn an_ack_without_its_echo_ends_the_redaction_without_an_identity() {
    let now = Instant::now();
    let mut session = ergo(now, REDACTION_CAPS);
    let id = redact(&mut session, now);

    let acked = play(&mut session, now, ACK);
    assert_eq!(acked.len(), 1, "{acked:?}");
    let response = response(&acked[0]);
    assert_eq!(response.id, id);
    assert_eq!(response.lines, []);
    assert_eq!(response.sent(), None);
    assert_eq!(response.error(), None);

    let late = play(&mut session, now, OWN_ECHO);
    assert_eq!(late.len(), 1, "{late:?}");
    assert_eq!(redaction_by(&late[0]), Some(Party::Me));
}

#[test]
fn an_own_echo_naming_no_message_is_not_tied() {
    let now = Instant::now();
    let mut session = ergo(now, REDACTION_CAPS);
    let id = redact(&mut session, now);

    let unmatched = play(
        &mut session,
        now,
        "@msgid=qsgqkhqcmnxm6x2dcricaf69f6;time=2026-11-16T07:39:12.791Z :papo!~u@fa4ww98eysn9a.irc REDACT #papo",
    );
    assert!(
        !unmatched
            .iter()
            .any(|event| matches!(event, Event::Response(_))),
        "{unmatched:?}"
    );
    let acked = play(&mut session, now, ACK);
    assert_eq!(acked.len(), 1, "{acked:?}");
    let response = response(&acked[0]);
    assert_eq!(response.id, id);
    assert_eq!(response.lines, []);
    assert_eq!(response.sent(), None);
}

#[test]
fn a_redaction_the_server_never_answers_times_out() {
    let now = Instant::now();
    let mut session = ergo(now, REDACTION_CAPS);
    let id = redact(&mut session, now);

    let deadline = session
        .poll_timeout()
        .expect("the redaction has a deadline");
    session.handle_timeout(deadline);
    assert!(matches!(
        session.poll_event(),
        Some(Event::RequestTimedOut { id: timed_out, .. }) if timed_out == id
    ));
    assert_eq!(play(&mut session, now, OWN_ECHO).len(), 1);
    let late = play(&mut session, now, ACK);
    assert!(
        !late.iter().any(|event| matches!(event, Event::Response(_))),
        "{late:?}"
    );
}

#[test]
fn only_the_first_own_echo_is_tied_and_a_repeat_does_not_end_the_request() {
    let now = Instant::now();
    let mut session = ergo(now, REDACTION_CAPS);
    let id = redact(&mut session, now);

    assert_eq!(play(&mut session, now, OWN_ECHO).len(), 1);
    let repeat = play(
        &mut session,
        now,
        &OWN_ECHO.replace("qsgqkhqcmnxm6x2dcricaf69f6", "8d3kz2mfqw7xbn5hyt4ce6rsja"),
    );
    assert_eq!(repeat.len(), 1, "{repeat:?}");
    assert_eq!(redaction_by(&repeat[0]), Some(Party::Me));
    let acked = play(&mut session, now, ACK);
    assert_eq!(acked.len(), 1, "{acked:?}");
    assert_eq!(response(&acked[0]).id, id);
    assert_eq!(response(&acked[0]).sent(), Some(own_echo_identity()));
}

#[test]
fn another_users_redaction_of_the_same_message_does_not_confirm_ours() {
    let now = Instant::now();
    let mut session = ergo(now, REDACTION_CAPS);
    let id = redact(&mut session, now);

    let foreign = play(&mut session, now, FOREIGN_REDACT);
    assert_eq!(foreign.len(), 1, "{foreign:?}");
    assert_eq!(redaction_by(&foreign[0]), Some(Party::Other));
    assert_eq!(play(&mut session, now, OWN_ECHO).len(), 1);
    let foreign = play(&mut session, now, FOREIGN_REDACT);
    assert_eq!(foreign.len(), 1, "{foreign:?}");

    let acked = play(&mut session, now, ACK);
    assert_eq!(acked.len(), 1, "{acked:?}");
    assert_eq!(response(&acked[0]).id, id);
    assert_eq!(response(&acked[0]).sent(), Some(own_echo_identity()));
}

#[test]
fn an_own_redaction_of_another_message_or_target_does_not_confirm_ours() {
    let now = Instant::now();
    for unrelated in [
        "@msgid=7wq9b3c2z5kxm8a4r6d1e0fyhu;time=2026-11-16T07:39:12.791Z :papo!~u@fa4ww98eysn9a.irc REDACT #papo kcq8m7mfjjd2vde7gcqaxxe2se superseded",
        "@msgid=7wq9b3c2z5kxm8a4r6d1e0fyhu;time=2026-11-16T07:39:12.791Z :papo!~u@fa4ww98eysn9a.irc REDACT #ergo b56bhkz4gmj7ig2jeg9hs7d2cs superseded",
        "@msgid=7wq9b3c2z5kxm8a4r6d1e0fyhu;time=2026-11-16T07:39:12.791Z :papo!~u@fa4ww98eysn9a.irc PRIVMSG #papo b56bhkz4gmj7ig2jeg9hs7d2cs",
        "@label=7;msgid=7wq9b3c2z5kxm8a4r6d1e0fyhu :papo!~u@fa4ww98eysn9a.irc REDACT #papo b56bhkz4gmj7ig2jeg9hs7d2cs superseded",
    ] {
        let mut session = ergo(now, REDACTION_CAPS);
        let id = redact(&mut session, now);
        let events = play(&mut session, now, unrelated);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::Response(_))),
            "{unrelated}: {events:?}"
        );
        assert_eq!(play(&mut session, now, OWN_ECHO).len(), 1, "{unrelated}");
        let acked = play(&mut session, now, ACK);
        assert_eq!(response(&acked[0]).id, id, "{unrelated}");
        assert_eq!(
            response(&acked[0]).sent(),
            Some(own_echo_identity()),
            "{unrelated}"
        );
    }
}

#[test]
fn a_refused_redaction_reports_its_failure_without_an_identity() {
    let now = Instant::now();
    for before in [None, Some(FOREIGN_REDACT), Some(OWN_ECHO)] {
        let mut session = ergo(now, REDACTION_CAPS);
        let id = redact(&mut session, now);
        if let Some(line) = before {
            assert_eq!(play(&mut session, now, line).len(), 1, "{line}");
        }
        let failed = play(&mut session, now, UNKNOWN_MSGID);
        assert_eq!(failed.len(), 1, "{before:?}: {failed:?}");
        let response = response(&failed[0]);
        assert_eq!(response.id, id);
        assert_eq!(response.sent(), None, "{before:?}");
        assert!(matches!(
            response.error(),
            Some(ServerError::Standard { ref code, .. }) if code == "UNKNOWN_MSGID"
        ));
    }
}

#[test]
fn without_echo_message_the_ack_completes_and_ergos_echo_stays_an_event() {
    let now = Instant::now();
    let mut session = ergo(
        now,
        "draft/message-redaction labeled-response message-tags server-time",
    );
    let id = redact(&mut session, now);

    let echoed = play(&mut session, now, OWN_ECHO);
    assert_eq!(echoed.len(), 1, "{echoed:?}");
    assert_eq!(redaction_by(&echoed[0]), Some(Party::Me));
    let acked = play(&mut session, now, ACK);
    assert_eq!(acked.len(), 1, "{acked:?}");
    assert_eq!(response(&acked[0]).id, id);
    assert_eq!(response(&acked[0]).sent(), None);
}

#[test]
fn an_echo_without_a_msgid_confirms_without_an_identity() {
    let now = Instant::now();
    let mut session = ergo(now, REDACTION_CAPS);
    let id = redact(&mut session, now);
    let echo = OWN_ECHO.replace("msgid=qsgqkhqcmnxm6x2dcricaf69f6;", "");
    assert_eq!(play(&mut session, now, &echo).len(), 1);
    let acked = play(&mut session, now, ACK);
    let response = response(&acked[0]);
    assert_eq!(response.id, id);
    assert_eq!(
        response.lines,
        [papo_proto::LineRef::parse(&echo).unwrap().to_owned()]
    );
    assert_eq!(response.sent(), None);
    assert_eq!(response.error(), None);
}
