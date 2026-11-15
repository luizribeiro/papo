//! Transcript coverage for the labelled echo of our own reactions.

use std::time::{Duration, Instant};

use papo_core::{Config, Event, RateLimit, Response, SentInfo, ServerError, Session};
use papo_proto::{Line, Numeric};

const ERGO_ISUPPORT: &str = ":ergo.test 005 papo CASEMAPPING=ascii CHANTYPES=# PREFIX=(qaohv)~&@%+ STATUSMSG=~&@%+ :are supported by this server";
const ECHO_CAPS: &str = "echo-message labeled-response message-tags server-time";

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

fn reaction(target: &str, tag: &str, emoji: &str) -> Line {
    Line::tagmsg(target)
        .with_tag("+draft/reply", "3gcfszmd5bzyy9g2evchgjqiv2")
        .with_tag(tag, emoji)
}

fn respond(session: &mut Session, now: Instant, line: &str) -> Response {
    while session.poll_transmit().is_some() {}
    session.handle_line(now, line.as_bytes());
    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("{line} should resolve the reaction request");
    };
    assert_eq!(session.poll_event(), None);
    response
}

fn sent(msgid: &str, time: &str) -> SentInfo {
    SentInfo {
        msgid: msgid.to_owned(),
        server_time: Some(time.to_owned()),
    }
}

#[test]
fn an_ergo_echo_confirms_a_reaction_with_its_msgid_and_time() {
    let now = Instant::now();
    let cases = [
        (
            "+draft/react",
            "#papo",
            "@msgid=ripxp4ezdb76is6ep47ic7rexa;time=2026-11-15T17:42:12.749Z;label=0;+draft/reply=3gcfszmd5bzyy9g2evchgjqiv2;+draft/react=👍 :papo!~u@7z29yeuh6n8ta.irc TAGMSG #papo",
            sent("ripxp4ezdb76is6ep47ic7rexa", "2026-11-15T17:42:12.749Z"),
        ),
        (
            "+draft/unreact",
            "#papo",
            "@time=2026-11-15T17:42:13.750Z;label=0;msgid=veexijhbhzbjwe74s63iww4haw;+draft/reply=3gcfszmd5bzyy9g2evchgjqiv2;+draft/unreact=👍 :papo!~u@7z29yeuh6n8ta.irc TAGMSG #papo",
            sent("veexijhbhzbjwe74s63iww4haw", "2026-11-15T17:42:13.750Z"),
        ),
        (
            "+draft/react",
            "@#papo",
            "@time=2026-11-15T17:42:14.751Z;label=0;msgid=6b4b6emqdux5z7beb724swiqhn;+draft/reply=3gcfszmd5bzyy9g2evchgjqiv2;+draft/react=👍 :papo!~u@7z29yeuh6n8ta.irc TAGMSG @#papo",
            sent("6b4b6emqdux5z7beb724swiqhn", "2026-11-15T17:42:14.751Z"),
        ),
    ];
    for (tag, target, echo, expected) in cases {
        let mut session = ergo(now, ECHO_CAPS);
        let id = session
            .request_silent(now, reaction(target, tag, "👍"))
            .unwrap();
        let response = respond(&mut session, now, echo);
        assert_eq!(response.id, id, "{tag} to {target}");
        assert_eq!(response.sent(), Some(expected), "{tag} to {target}");
        assert_eq!(response.error(), None, "{tag} to {target}");
        assert_eq!(response.lines[0].as_ref().param(0), Some(target));
    }
}

#[test]
fn a_reaction_without_its_echo_has_no_identity() {
    let now = Instant::now();
    let cases = [
        ("labeled-response message-tags", "@label=0 :ergo.test ACK"),
        (
            ECHO_CAPS,
            "@time=2026-11-15T17:42:12.749Z;label=0;+draft/reply=3gcfszmd5bzyy9g2evchgjqiv2;+draft/react=👍 :papo!~u@7z29yeuh6n8ta.irc TAGMSG #papo",
        ),
    ];
    for (caps, reply) in cases {
        let mut session = ergo(now, caps);
        session
            .request_silent(now, reaction("#papo", "+draft/react", "👍"))
            .unwrap();
        let response = respond(&mut session, now, reply);
        assert_eq!(response.sent(), None, "{reply}");
        assert_eq!(response.error(), None, "{reply}");
    }
}

#[test]
fn a_refused_reaction_reports_the_error_without_an_identity() {
    let now = Instant::now();
    let mut session = ergo(now, ECHO_CAPS);
    session
        .request_silent(now, reaction("nosuchnick", "+draft/react", "👀"))
        .unwrap();
    let response = respond(
        &mut session,
        now,
        "@time=2026-11-15T17:42:15.752Z;label=0 :ergo.test 401 papo nosuchnick :No such nick",
    );
    assert_eq!(response.sent(), None);
    assert!(matches!(
        response.error(),
        Some(ServerError::Numeric { numeric, .. }) if numeric == Numeric::ERR_NOSUCHNICK
    ));
}

#[test]
fn a_dropped_reaction_times_out() {
    let now = Instant::now();
    let mut session = ergo(now, ECHO_CAPS);
    let id = session
        .request_silent(now, reaction("#papo", "+draft/react", "👍"))
        .unwrap();
    while session.poll_transmit().is_some() {}
    let deadline = session.poll_timeout().expect("the reaction has a deadline");
    assert!(deadline > now + Duration::from_secs(1));
    session.handle_timeout(deadline);
    assert!(matches!(
        session.poll_event(),
        Some(Event::RequestTimedOut { id: timed_out, .. }) if timed_out == id
    ));
}
