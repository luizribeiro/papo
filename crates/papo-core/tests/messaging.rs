//! Transcript coverage for incoming modern messaging commands.

mod support;

use std::time::Instant;

use papo_core::{
    Authorship, Config, Conversation, CtcpReplies, Event, RateLimit, Session, TypingReason,
};
use papo_proto::{Line, Reaction, Typing};
use support::Transcript;

fn tag_message(line: &str) -> papo_core::TagMessage {
    let mut session = Session::new(Config::default());
    session.handle_line(Instant::now(), line.as_bytes());
    let Some(Event::TagMessage(message)) = session.poll_event() else {
        panic!("expected tag message")
    };
    message
}

fn as_message(tag: &papo_core::TagMessage) -> Line {
    let address = match &tag.conversation {
        Conversation::Channel(channel) => tag
            .status
            .iter()
            .flatten()
            .copied()
            .chain(channel.chars())
            .collect(),
        Conversation::Private(nick) => nick.to_string(),
        Conversation::Server => "*".to_owned(),
        Conversation::Broadcast(mask) => mask.to_string(),
        _ => panic!("unexpected conversation kind"),
    };
    let mut message = Line::tagmsg(address);
    message = message.with_source(&tag.actor);
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
        let mut session = Session::new(Config::default());
        session.handle_line(
            Instant::now(),
            format!("@+typing={value} :alice!~a@cloak TAGMSG #papo").as_bytes(),
        );
        let Some(Event::TypingChanged(change)) = session.poll_event() else {
            panic!("expected typed notification")
        };
        assert_eq!(change.actor, "alice!~a@cloak");
        assert_eq!(change.authorship, Authorship::Other);
        assert!(
            matches!(change.conversation, Conversation::Channel(ref channel) if channel == "#papo")
        );
        assert_eq!(change.status, None);
        assert_eq!(change.state, expected);
        assert_eq!(change.reason, TypingReason::Notified);
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
fn classifies_status_and_private_tag_messages() {
    let mut session = Session::new(Config::default().nick("papo"));
    session.handle_line(
        Instant::now(),
        b":ergo.test 005 papo CHANTYPES=# STATUSMSG=@+ :are supported",
    );
    session.handle_line(
        Instant::now(),
        b"@+typing=active :alice!~a@cloak TAGMSG @#ops",
    );
    let Some(Event::TypingChanged(channel)) = session.poll_event() else {
        panic!("expected channel typing event")
    };
    assert!(matches!(channel.conversation, Conversation::Channel(ref name) if name == "#ops"));
    assert_eq!(channel.status, Some(vec!['@']));

    session.handle_line(Instant::now(), b"@+typing=done :alice!~a@cloak TAGMSG papo");
    let Some(Event::TypingChanged(private)) = session.poll_event() else {
        panic!("expected private typing event")
    };
    assert!(matches!(private.conversation, Conversation::Private(ref nick) if nick == "alice"));
    assert_eq!(private.status, None);

    session.handle_line(
        Instant::now(),
        b"@+typing=active :papo!~papo@localhost TAGMSG #ops",
    );
    let Some(Event::TypingChanged(own)) = session.poll_event() else {
        panic!("expected own typing event")
    };
    assert_eq!(own.authorship, Authorship::Own);
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
        assert_eq!(redacted.actor, "alice!~a@cloak");
        assert_eq!(redacted.authorship, Authorship::Other);
        assert!(
            matches!(redacted.conversation, Conversation::Channel(ref channel) if channel == "#papo")
        );
        assert_eq!(redacted.status, None);
        assert!(matches!(redacted.msgid.as_str(), "RKi9Cw" | "8sKY2A"));
        assert_eq!(redacted.reason.as_deref(), expected_reason);
    }
}

#[test]
fn leaves_actorless_activity_lines_untyped() {
    Transcript::new(Config::default())
        .server("@+typing=active TAGMSG #papo")
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("TAGMSG")))
        .server("REDACT #papo RKi9Cw :Removed by author")
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("REDACT")));
}

fn wire(session: &mut Session) -> Option<String> {
    session
        .poll_transmit()
        .map(|line| String::from_utf8(line).unwrap())
}

fn connected_ctcp(replies: CtcpReplies) -> Transcript {
    Transcript::new(Config::default().wanted_caps([] as [&str; 0]).ctcp(replies))
        .connect_default()
        .server(":ergo.test CAP * LS :")
        .client("CAP END")
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
}

#[test]
fn answers_supported_ctcp_queries_and_applies_safety_rules() {
    let now = Instant::now();
    let replies = CtcpReplies::default()
        .version(Some("papo test build"))
        .rate(RateLimit::unlimited());
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

    session.handle_line(now, b":dave!~d@cloak PRIVMSG papo :\x01SOURCE\x01");
    assert_eq!(wire(&mut session), None);
    session.handle_line(now, b":erin!~e@cloak PRIVMSG papo :\x01CLIENTINFO\x01");
    assert_eq!(wire(&mut session), None);
    session.handle_line(now, b":frank!~f@cloak PRIVMSG papo :\x01PING\x01");
    assert_eq!(
        wire(&mut session).as_deref(),
        Some("NOTICE frank \x01PING\x01\r\n")
    );
    let default_version = format!("papo {}", env!("CARGO_PKG_VERSION"));
    assert_ctcp_combination(CtcpReplies::default(), Some(&default_version), true);
}

fn assert_ctcp_combination(replies: CtcpReplies, version: Option<&str>, ping: bool) {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .rate_limit(RateLimit::unlimited())
            .ctcp(replies.rate(RateLimit::unlimited())),
    );
    session.handle_line(now, b":alice!~a@cloak PRIVMSG papo :\x01VERSION\x01");
    assert_eq!(
        wire(&mut session),
        version.map(|text| format!("NOTICE alice :\x01VERSION {text}\x01\r\n"))
    );
    session.handle_line(now, b":bob!~b@cloak PRIVMSG papo :\x01PING 1731513600\x01");
    assert_eq!(
        wire(&mut session),
        ping.then(|| "NOTICE bob :\x01PING 1731513600\x01\r\n".to_owned())
    );
}

#[test]
fn playback_batches_do_not_trigger_automatic_ctcp_replies() {
    let replies = CtcpReplies::default()
        .version(Some("papo test build"))
        .rate(RateLimit::unlimited());
    Transcript::new(Config::default().nick("papo").ctcp(replies))
        .server(":irc.example.net BATCH +zncplayback znc.in/playback")
        .no_client()
        .server(
            "@batch=zncplayback :alice!~alice@users.example.net PRIVMSG papo :\x01VERSION\x01",
        )
        .no_client()
        .event(|event| {
            matches!(event, Event::Line(line) if line.batch_type() == Some("znc.in/playback"))
        })
        .server(":irc.example.net BATCH -zncplayback")
        .no_client()
        .server("@label=expired :irc.example.net BATCH +labelled znc.in/playback")
        .server(
            "@batch=labelled :bob!~bob@users.example.net PRIVMSG papo :\x01VERSION\x01",
        )
        .no_client()
        .server(":irc.example.net BATCH -labelled")
        .no_client()
        .event(|event| {
            matches!(event, Event::Line(line) if line.batch_type() == Some("znc.in/playback"))
        })
        .server(":alice!~alice@users.example.net PRIVMSG papo :\x01VERSION\x01")
        .client("NOTICE alice :\x01VERSION papo test build\x01")
        .event(|event| matches!(event, Event::Line(line) if line.batch_type().is_none()));
}

#[test]
fn configures_every_ping_and_version_combination_from_both_starting_points() {
    let default_version = format!("papo {}", env!("CARGO_PKG_VERSION"));
    for (replies, version, ping) in [
        (CtcpReplies::default(), Some(default_version.as_str()), true),
        (
            CtcpReplies::default().ping(false),
            Some(default_version.as_str()),
            false,
        ),
        (CtcpReplies::default().version(None), None, true),
        (
            CtcpReplies::default().version(None).ping(false),
            None,
            false,
        ),
        (CtcpReplies::off(), None, false),
        (CtcpReplies::off().ping(true), None, true),
        (
            CtcpReplies::off().version(Some("ferrobot 2.3")),
            Some("ferrobot 2.3"),
            false,
        ),
        (
            CtcpReplies::off().ping(true).version(Some("ferrobot 2.3")),
            Some("ferrobot 2.3"),
            true,
        ),
    ] {
        assert_ctcp_combination(replies, version, ping);
    }
}

#[test]
fn reports_ctcp_queries_replies_and_actions_while_auto_answering() {
    let replies = CtcpReplies::default()
        .version(Some("papo test build"))
        .rate(RateLimit::unlimited());
    Transcript::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited())
            .ctcp(replies),
    )
    .server("@time=2026-10-20T12:34:56.000Z :alice!~a@cloak PRIVMSG papo :\x01VERSION\x01")
    .client("NOTICE alice :\x01VERSION papo test build\x01")
    .event(|event| matches!(event, Event::Line(line) if line.as_ref().param(1) == Some("\x01VERSION\x01")))
    .server(":bob!~b@cloak PRIVMSG papo :\x01PING 1731513600\x01")
    .client("NOTICE bob :\x01PING 1731513600\x01")
    .event(|event| matches!(event, Event::Line(line) if line.as_ref().param(1) == Some("\x01PING 1731513600\x01")))
    .server(":carol!~c@cloak PRIVMSG #papo :\x01ACTION reviews the patch\x01")
    .no_client()
    .event(|event| matches!(event, Event::Line(line) if line.as_ref().param(1) == Some("\x01ACTION reviews the patch\x01")))
    .server(":dan!~d@cloak PRIVMSG #papo :\x01ACTION tests a split message")
    .no_client()
    .event(|event| matches!(event, Event::Line(line) if line.as_ref().param(1) == Some("\x01ACTION tests a split message")))
    .server(":soju.example NOTICE papo :\x01VERSION soju 0.8.2\x01")
    .no_client()
    .event(|event| matches!(event, Event::Line(line) if line.as_ref().param(1) == Some("\x01VERSION soju 0.8.2\x01")));
}

#[test]
fn does_not_answer_own_ctcp_queries() {
    Transcript::new(Config::default().nick("papo"))
        .server(":papo!~p@cloak PRIVMSG papo :\x01VERSION\x01")
        .no_client()
        .event(|event| matches!(event, Event::Line(_)))
        .no_event();

    Transcript::new(Config::default().nick("Papo[bot]"))
        .server(":papo{bot}!~p@cloak PRIVMSG Papo[bot] :\x01VERSION\x01")
        .no_client()
        .event(|event| matches!(event, Event::Line(_)))
        .no_event();
}

#[test]
fn disabled_ctcp_replies_answer_nothing() {
    let now = Instant::now();
    let replies = CtcpReplies::off();
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
    session.handle_line(now, b":bob!~b@cloak PRIVMSG papo :\x01VERSION\x01");
    assert_eq!(wire(&mut session), None);
    session.handle_line(now, b":carol!~c@cloak PRIVMSG papo :\x01TIME\x01");
    assert_eq!(wire(&mut session), None);
    session.handle_line(now, b":dan!~d@cloak PRIVMSG papo :\x01SOURCE\x01");
    assert_eq!(wire(&mut session), None);
}

#[test]
fn disabling_version_still_answers_ping() {
    connected_ctcp(CtcpReplies::default().version(None))
        .server(":alice!~a@cloak PRIVMSG papo :\x01VERSION\x01")
        .no_client()
        .event(|event| matches!(event, Event::Line(_)))
        .server(":bob!~b@cloak PRIVMSG papo :\x01PING 1731513600\x01")
        .client("NOTICE bob :\x01PING 1731513600\x01")
        .event(|event| matches!(event, Event::Line(_)));
}

#[test]
fn channel_ctcp_queries_are_reported_without_automatic_replies() {
    connected_ctcp(CtcpReplies::default().rate(RateLimit::unlimited()))
        .server(":ergo.test 005 papo STATUSMSG=@+ :are supported")
        .server(":alice!~a@cloak PRIVMSG #papo :\x01PING 1731513600\x01")
        .no_client()
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().param(1) == Some("\x01PING 1731513600\x01")))
        .server(":bob!~b@cloak PRIVMSG @#papo :\x01VERSION\x01")
        .no_client()
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().param(0) == Some("@#papo")));
}

#[test]
fn replay_batches_report_ctcp_queries_without_automatic_replies() {
    connected_ctcp(CtcpReplies::default().rate(RateLimit::unlimited()))
        .server(":soju.example BATCH +backlog chathistory papo")
        .server("@batch=backlog :alice!~a@cloak PRIVMSG papo :\x01VERSION\x01")
        .no_client()
        .server(":soju.example BATCH -backlog")
        .no_client()
        .event(|event| matches!(event, Event::History { items, .. } if items.len() == 1))
        .server(":znc.example BATCH +playback znc.in/playback")
        .server("@batch=playback :bob!~b@cloak PRIVMSG papo :\x01PING 1731513600\x01")
        .no_client()
        .event(|event| matches!(event, Event::Line(line) if line.batch_type() == Some("znc.in/playback")))
        .server(":znc.example BATCH -playback")
        .no_client()
        .server("@label=missing :ergo.test BATCH +reply labeled-response")
        .server("@batch=reply :carol!~c@cloak PRIVMSG papo :\x01VERSION\x01")
        .server(":ergo.test BATCH -reply")
        .no_client()
        .event(|event| matches!(event, Event::Line(line) if line.batch_type() == Some("labeled-response")));
}

#[test]
fn connectivity_batches_keep_live_ctcp_queries_answerable() {
    connected_ctcp(CtcpReplies::default().rate(RateLimit::unlimited()))
        .server(":ergo.test BATCH +joined netjoin irc1.example irc2.example")
        .server("@batch=joined :alice!~a@cloak PRIVMSG papo :\x01PING 1731513600\x01")
        .no_client()
        .server(":ergo.test BATCH -joined")
        .client("NOTICE alice :\x01PING 1731513600\x01")
        .event(|event| matches!(event, Event::Netjoin { .. }))
        .event(|event| matches!(event, Event::Line(line) if line.batch_type() == Some("netjoin")))
        .server(":ergo.test BATCH +split netsplit irc1.example irc2.example")
        .server("@batch=split :bob!~b@cloak PRIVMSG papo :\x01VERSION\x01")
        .no_client()
        .server(":ergo.test BATCH -split")
        .client(&format!(
            "NOTICE bob :\x01VERSION papo {}\x01",
            env!("CARGO_PKG_VERSION")
        ))
        .event(|event| matches!(event, Event::Netsplit { .. }))
        .event(|event| matches!(event, Event::Line(line) if line.batch_type() == Some("netsplit")));
}

#[test]
fn caps_ctcp_replies_across_several_senders() {
    let replies = CtcpReplies::default()
        .version(Some("papo test build"))
        .rate(RateLimit::new(2, std::time::Duration::from_secs(2)).unwrap());
    connected_ctcp(replies)
        .server(":alice!~a@cloak PRIVMSG papo :\x01VERSION\x01")
        .client("NOTICE alice :\x01VERSION papo test build\x01")
        .event(|event| matches!(event, Event::Line(_)))
        .server(":bob!~b@cloak PRIVMSG papo :\x01PING 1731513600\x01")
        .client("NOTICE bob :\x01PING 1731513600\x01")
        .event(|event| matches!(event, Event::Line(_)))
        .server(":carol!~c@cloak PRIVMSG papo :\x01VERSION\x01")
        .no_client()
        .event(|event| matches!(event, Event::Line(_)))
        .advance(std::time::Duration::from_secs(1))
        .server(":dan!~d@cloak PRIVMSG papo :\x01PING 1731513601\x01")
        .no_client()
        .event(|event| matches!(event, Event::Line(_)))
        .advance(std::time::Duration::from_secs(1))
        .server(":erin!~e@cloak PRIVMSG papo :\x01VERSION\x01")
        .client("NOTICE erin :\x01VERSION papo test build\x01")
        .event(|event| matches!(event, Event::Line(_)));
}

#[test]
fn ctcp_source_bound_evicts_only_when_full() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .rate_limit(RateLimit::unlimited())
            .ctcp(CtcpReplies::default().rate(RateLimit::unlimited())),
    );
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
