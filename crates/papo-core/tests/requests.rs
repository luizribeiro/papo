//! Transcript coverage for labelled request correlation.

use std::time::Instant;

use papo_core::{Config, Event, RateLimit, RequestError, RequestId, ServerError, Session};
use papo_proto::{Message, MessageRef, Numeric};

fn labelled_session(now: Instant) -> Session {
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :labeled-response");
    session.handle_line(now, b"CAP * ACK :labeled-response");
    while session.poll_transmit().is_some() {}
    session
}

fn request(session: &mut Session, now: Instant, message: Message) -> RequestId {
    session.request(now, message).unwrap()
}

#[test]
fn resolves_a_labelled_echo_and_an_ack() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let echo = request(
        &mut session,
        now,
        Message::privmsg("#papo", "hello from papo"),
    );
    let mode = request(&mut session, now, Message::new("MODE", ["#papo", "+i"]));
    session.handle_line(
        now,
        b"@label=0;msgid=RKi9Cw :papo!~papo@localhost PRIVMSG #papo :hello from papo",
    );
    session.handle_line(now, b"@label=1 :ergo.test ACK");

    assert!(matches!(
        session.poll_event(),
        Some(Event::Response(response))
            if response.id == echo
                && response.lines.len() == 1
                && response.lines[0].as_ref().tag("msgid").as_deref() == Some("RKi9Cw")
    ));
    assert!(matches!(
        session.poll_event(),
        Some(Event::Response(response)) if response.id == mode && response.lines.is_empty()
    ));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn labelled_join_updates_state_before_resolving() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = request(&mut session, now, Message::join("#papo"));
    session.handle_line(
        now,
        b"@label=0 :papo!~papo@localhost JOIN #papo papo :Papo Bot",
    );

    assert!(matches!(
        session.poll_event(),
        Some(Event::Joined { channel, nick }) if channel == "#papo" && nick == "papo"
    ));
    assert!(matches!(
        session.poll_event(),
        Some(Event::Response(response)) if response.id == id && response.lines.len() == 1
    ));
    assert!(session.state().channel("#papo").is_some());
}

#[test]
fn resolves_a_recorded_ergo_whois_batch_once() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = request(&mut session, now, Message::new("WHOIS", ["alice"]));
    session.handle_line(now, b"@label=0 :ergo.test BATCH +whois-42 labeled-response");
    for line in [
        "@batch=whois-42 :ergo.test 311 papo alice ~alice localhost * :Alice Example",
        "@batch=whois-42 :ergo.test 312 papo alice ergo.test :Ergo Test Network",
        "@batch=whois-42 :ergo.test 318 papo alice :End of /WHOIS list",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    assert_eq!(session.poll_event(), None);
    session.handle_line(now, b":ergo.test BATCH -whois-42");

    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("expected one response")
    };
    assert_eq!(response.id, id);
    assert_eq!(response.lines.len(), 3);
    assert!(response.lines[0].as_ref().command_is("311"));
    assert!(response.lines[2].as_ref().command_is("318"));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn links_nested_chathistory_to_its_response() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = request(
        &mut session,
        now,
        Message::new("CHATHISTORY", ["LATEST", "#papo", "*", "50"]),
    );
    session.handle_line(now, b"@label=0 :ergo.test BATCH +reply labeled-response");
    session.handle_line(
        now,
        b"@batch=reply;draft/chathistory-end :ergo.test BATCH +history chathistory #papo",
    );
    session.handle_line(
        now,
        b"@batch=history;time=2026-10-15T12:00:01.000Z;msgid=RKi9Cw :alice!~a@localhost PRIVMSG #papo :earlier",
    );
    session.handle_line(now, b"@batch=reply :ergo.test BATCH -history");
    session.handle_line(now, b":ergo.test BATCH -reply");

    assert!(matches!(
        session.poll_event(),
        Some(Event::History { target, messages, end })
            if target == "#papo" && messages.len() == 1 && end
    ));
    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("expected correlated history response")
    };
    assert_eq!(response.id, id);
    assert_eq!(response.lines.len(), 1);
    let history = response.history.expect("history reference");
    assert_eq!(history.target, "#papo");
    assert_eq!(history.messages.len(), 1);
    assert!(history.end);
}

#[test]
fn labelled_fail_and_numeric_responses_expose_errors() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    request(
        &mut session,
        now,
        Message::new("CHATHISTORY", ["LATEST", "#papo", "bad", "50"]),
    );
    request(&mut session, now, Message::new("WHOIS", ["missing"]));
    session.handle_line(now, b"@label=0 :ergo.test BATCH +failure labeled-response");
    session.handle_line(
        now,
        b"@batch=failure :ergo.test FAIL CHATHISTORY INVALID_PARAMS #papo bad :Invalid message reference",
    );
    session.handle_line(now, b":ergo.test BATCH -failure");
    session.handle_line(
        now,
        b"@label=1 :ergo.test 401 papo missing :No such nick/channel",
    );

    let Some(Event::Response(fail)) = session.poll_event() else {
        panic!("expected standard failure response")
    };
    assert_eq!(
        fail.error(),
        Some(ServerError::Standard {
            command: "CHATHISTORY".into(),
            code: "INVALID_PARAMS".into(),
            context: vec!["#papo".into(), "bad".into()],
            description: "Invalid message reference".into(),
        })
    );
    let Some(Event::Response(numeric)) = session.poll_event() else {
        panic!("expected numeric failure response")
    };
    assert_eq!(
        numeric.error(),
        Some(ServerError::Numeric {
            numeric: Numeric::ERR_NOSUCHNICK,
            params: vec![
                "papo".into(),
                "missing".into(),
                "No such nick/channel".into()
            ],
        })
    );
}

#[test]
fn times_out_and_releases_a_label_for_late_delivery() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = request(&mut session, now, Message::new("WHOIS", ["alice"]));
    assert_eq!(
        session.poll_timeout(),
        Some(now + std::time::Duration::from_secs(30))
    );
    session.handle_timeout(now + std::time::Duration::from_secs(29));
    assert_eq!(session.poll_event(), None);
    session.handle_timeout(now + std::time::Duration::from_secs(30));
    assert!(
        matches!(session.poll_event(), Some(Event::RequestTimedOut { id: expired }) if expired == id)
    );
    session.handle_line(
        now + std::time::Duration::from_secs(31),
        b"@label=0 :ergo.test 401 papo alice :No such nick/channel",
    );
    assert!(
        matches!(session.poll_event(), Some(Event::Message(message)) if message.as_ref().command_is("401"))
    );
}

#[test]
fn aborts_pending_requests_on_disconnect() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = request(&mut session, now, Message::new("WHOIS", ["alice"]));
    session.handle_disconnected(now);
    assert!(
        matches!(session.poll_event(), Some(Event::RequestAborted { id: aborted }) if aborted == id)
    );
}

#[test]
fn bounds_pending_requests_at_the_literal_limit() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    for nick in 0..256 {
        session
            .request(now, Message::new("WHOIS", [format!("user{nick}")]))
            .unwrap();
    }
    assert_eq!(
        session.request(now, Message::new("WHOIS", ["overflow"])),
        Err(RequestError::PendingLimit)
    );
}

#[test]
fn generates_one_thousand_distinct_short_alphanumeric_labels() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let mut labels = std::collections::BTreeSet::new();
    for nick in 0..1_000 {
        session
            .request(now, Message::new("WHOIS", [format!("user{nick}")]))
            .unwrap();
        let line = String::from_utf8(session.poll_transmit().unwrap()).unwrap();
        let message = MessageRef::parse(&line).unwrap();
        let label = message.tag("label").unwrap().into_owned();
        assert!(label.len() <= 64);
        assert!(label.bytes().all(|byte| byte.is_ascii_alphanumeric()));
        assert!(labels.insert(label.clone()));
        session.handle_line(now, format!("@label={label} :ergo.test ACK").as_bytes());
        assert!(matches!(session.poll_event(), Some(Event::Response(_))));
    }
}
