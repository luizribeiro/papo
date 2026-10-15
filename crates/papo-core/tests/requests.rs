//! Transcript coverage for labelled request correlation.

use std::time::Instant;

use papo_core::{Config, Event, RateLimit, RequestId, Session};
use papo_proto::Message;

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
