//! Transcript coverage for incoming modern messaging commands.

use std::time::Instant;

use papo_core::{Config, CtcpReplies, Event, RateLimit, Session};
use papo_proto::{Message, Reaction, Typing};

fn tag_message(line: &str) -> papo_core::TagMessage {
    let mut session = Session::new(Config::default());
    session.handle_line(Instant::now(), line.as_bytes());
    let Some(Event::TagMessage(message)) = session.poll_event() else {
        panic!("expected tag message")
    };
    message
}

fn as_message(tag: &papo_core::TagMessage) -> Message {
    let mut message = Message::tagmsg(&tag.target);
    if let Some(source) = &tag.source {
        message = message.with_source(source);
    }
    for (key, value) in &tag.tags {
        message = message.with_tag(key, value.as_deref().unwrap_or_default());
    }
    message
}

#[test]
fn reports_active_paused_and_done_typing() {
    for (value, expected) in [
        ("active", Typing::Active),
        ("paused", Typing::Paused),
        ("done", Typing::Done),
    ] {
        let tag = tag_message(&format!("@+typing={value} :alice!~a@cloak TAGMSG #papo"));
        assert_eq!(tag.source.as_deref(), Some("alice!~a@cloak"));
        assert_eq!(tag.target, "#papo");
        assert_eq!(Typing::parse(&as_message(&tag).as_ref()), Some(expected));
    }
}

#[test]
fn reports_react_and_unreact_with_a_reply() {
    for (tag_name, expected_react, expected_unreact) in [
        ("+draft/react", Some("👍"), None),
        ("+draft/unreact", None, Some("👍")),
    ] {
        let tag = tag_message(&format!(
            "@{tag_name}=👍;+reply=RKi9Cw :bob!~b@cloak TAGMSG #papo"
        ));
        let owned = as_message(&tag);
        let reaction = Reaction::parse(&owned.as_ref()).unwrap();
        assert_eq!(reaction.react.as_deref(), expected_react);
        assert_eq!(reaction.unreact.as_deref(), expected_unreact);
        assert_eq!(reaction.reply_to, "RKi9Cw");
    }
}

#[test]
fn reports_tagmsg_without_known_tags() {
    let tag = tag_message("@+example.org/read=cursor-42 :soju.example TAGMSG papo");
    assert_eq!(
        tag.tags,
        [("+example.org/read".into(), Some("cursor-42".into()))]
    );
}

#[test]
fn reports_redactions_with_and_without_a_reason() {
    for (line, expected_reason) in [
        (
            ":alice!~a@cloak REDACT #papo RKi9Cw :Removed by author",
            Some("Removed by author"),
        ),
        (":alice!~a@cloak REDACT #papo 8sKY2A", None),
    ] {
        let mut session = Session::new(Config::default());
        session.handle_line(Instant::now(), line.as_bytes());
        let Some(Event::Redacted(redacted)) = session.poll_event() else {
            panic!("expected redaction")
        };
        assert_eq!(redacted.source.as_deref(), Some("alice!~a@cloak"));
        assert_eq!(redacted.target, "#papo");
        assert!(matches!(redacted.msgid.as_str(), "RKi9Cw" | "8sKY2A"));
        assert_eq!(redacted.reason.as_deref(), expected_reason);
    }
}

fn wire(session: &mut Session) -> Option<String> {
    session
        .poll_transmit()
        .map(|line| String::from_utf8(line).unwrap())
}

#[test]
fn answers_supported_ctcp_queries_and_applies_safety_rules() {
    let now = Instant::now();
    let replies = CtcpReplies {
        version: Some("papo test build".into()),
        ping: true,
        time: None,
        source: Some("https://example.invalid/papo".into()),
    };
    let mut session = Session::new(
        Config::default()
            .rate_limit(RateLimit::unlimited())
            .ctcp(replies),
    );

    session.handle_line(now, b":alice!~a@cloak PRIVMSG papo :\x01VERSION\x01");
    assert_eq!(
        wire(&mut session).as_deref(),
        Some("NOTICE alice :\x01VERSION papo test build\x01\r\n")
    );
    session.handle_line(
        now,
        b":alice!~a@cloak PRIVMSG papo :\x01PING 1731513600\x01",
    );
    assert_eq!(wire(&mut session), None);
    session.handle_line(
        now + std::time::Duration::from_secs(2),
        b":alice!~a@cloak PRIVMSG papo :\x01PING 1731513600\x01",
    );
    assert_eq!(
        wire(&mut session).as_deref(),
        Some("NOTICE alice :\x01PING 1731513600\x01\r\n")
    );

    session.handle_line(now, b":bob!~b@cloak PRIVMSG papo :\x01TIME\x01");
    session.handle_line(now, b":carol!~c@cloak NOTICE papo :\x01VERSION\x01");
    assert_eq!(wire(&mut session), None);

    session.handle_line(now, b":dave!~d@cloak PRIVMSG #papo :\x01SOURCE\x01");
    assert_eq!(
        wire(&mut session).as_deref(),
        Some("NOTICE dave :\x01SOURCE https://example.invalid/papo\x01\r\n")
    );
}

#[test]
fn honors_disabled_ping_and_time_replies() {
    let now = Instant::now();
    let replies = CtcpReplies {
        version: None,
        ping: false,
        time: None,
        source: None,
    };
    let mut session = Session::new(
        Config::default()
            .rate_limit(RateLimit::unlimited())
            .ctcp(replies),
    );
    session.handle_line(
        now,
        b":alice!~a@cloak PRIVMSG papo :\x01PING 1731513600\x01",
    );
    assert_eq!(wire(&mut session), None);
    session.handle_line(now, b":bob!~b@cloak PRIVMSG papo :\x01TIME\x01");
    assert_eq!(wire(&mut session), None);
}

#[test]
fn answers_time_with_the_injected_wall_clock_value() {
    let now = Instant::now();
    let replies = CtcpReplies {
        version: None,
        ping: false,
        time: Some("Thu Oct 15 16:30:00 2026 EDT".into()),
        source: None,
    };
    let mut session = Session::new(
        Config::default()
            .rate_limit(RateLimit::unlimited())
            .ctcp(replies),
    );
    session.handle_line(now, b":alice!~a@cloak PRIVMSG papo :\x01TIME\x01");
    assert_eq!(
        wire(&mut session).as_deref(),
        Some("NOTICE alice :\x01TIME Thu Oct 15 16:30:00 2026 EDT\x01\r\n")
    );
}

#[test]
fn ctcp_source_bound_evicts_only_when_full() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    for index in 0..1024 {
        session.handle_line(
            now,
            format!(":user{index}!~u@cloak PRIVMSG papo :\x01VERSION\x01").as_bytes(),
        );
        assert!(wire(&mut session).is_some());
    }
    session.handle_line(now, b":user0!~u@cloak PRIVMSG papo :\x01VERSION\x01");
    assert_eq!(wire(&mut session), None);

    session.handle_line(now, b":user1024!~u@cloak PRIVMSG papo :\x01VERSION\x01");
    assert!(wire(&mut session).is_some());
    session.handle_line(now, b":user0!~u@cloak PRIVMSG papo :\x01VERSION\x01");
    assert!(wire(&mut session).is_some());
}
