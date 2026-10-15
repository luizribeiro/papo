//! Transcript coverage for labelled request correlation.

use std::{collections::BTreeMap, time::Instant};

use papo_core::{
    Config, Event, RateLimit, RequestError, RequestId, ResponseMatcher, ServerError, Session,
};
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
fn matches_an_unlabelled_whois_through_its_end_numeric() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    let id = session
        .request_with_matcher(
            now,
            Message::new("WHOIS", ["Alice["]),
            ResponseMatcher::Whois("Alice[".into()),
        )
        .unwrap();
    session.handle_line(
        now,
        b":ergo.test 311 papo alice{ ~alice localhost * :Alice Example",
    );
    session.handle_line(now, b":ergo.test 318 papo alice{ :End of /WHOIS list");

    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("expected a response")
    };
    assert_eq!((response.id, response.lines.len()), (id, 2));
}

#[test]
fn unlabelled_join_waits_for_channel_sync() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited()),
    );
    let id = session
        .request_with_matcher(
            now,
            Message::join("#papo"),
            ResponseMatcher::Join("#papo".into()),
        )
        .unwrap();
    for line in [
        ":papo!papo@localhost JOIN #papo * :Papo Client",
        ":ergo.test 353 papo = #papo :@papo alice",
        ":ergo.test 366 papo #papo :End of /NAMES list",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    assert!(
        std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::Response(response) if response.id == id))
    );
}

fn assert_unlabelled(
    session: &mut Session,
    message: Message,
    matcher: ResponseMatcher,
    lines: &[&str],
) {
    let now = Instant::now();
    let id = session.request_with_matcher(now, message, matcher).unwrap();
    for line in lines {
        session.handle_line(now, line.as_bytes());
    }
    assert!(
        std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::Response(response) if response.id == id))
    );
}

fn response_id(session: &mut Session) -> RequestId {
    std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::Response(response) => Some(response.id),
            _ => None,
        })
        .unwrap()
}

fn response_lines(session: &mut Session) -> (BTreeMap<RequestId, Vec<Message>>, Vec<Message>) {
    let mut responses = BTreeMap::new();
    let mut messages = Vec::new();
    while let Some(event) = session.poll_event() {
        match event {
            Event::Response(response) => {
                responses.insert(response.id, response.lines);
            }
            Event::Message(message) => messages.push(message.into_message()),
            _ => {}
        }
    }
    (responses, messages)
}

fn unlabelled_whois(session: &mut Session, now: Instant, nick: &str) -> RequestId {
    session
        .request_with_matcher(
            now,
            Message::new("WHOIS", [nick]),
            ResponseMatcher::Whois(nick.into()),
        )
        .unwrap()
}

#[test]
fn matches_other_unlabelled_request_boundaries() {
    let mut session = Session::new(Config::default().nick("papo"));
    assert_unlabelled(
        &mut session,
        Message::new("WHO", ["#papo"]),
        ResponseMatcher::Who("#papo".into()),
        &[
            ":ergo.test 352 papo #papo ~alice host ergo.test alice H@ :0 Alice Example",
            ":ergo.test 315 papo #papo :End of WHO list",
        ],
    );
    assert_unlabelled(
        &mut session,
        Message::new("LIST", Vec::<String>::new()),
        ResponseMatcher::List,
        &[
            ":ergo.test 321 papo Channel :Users Name",
            ":ergo.test 322 papo #papo 42 :Ergonomic IRC",
            ":ergo.test 323 papo :End of /LIST",
        ],
    );
    assert_unlabelled(
        &mut session,
        Message::new("PART", ["#papo"]),
        ResponseMatcher::Part("#papo".into()),
        &[":papo!papo@localhost PART #papo :Leaving"],
    );
    assert_unlabelled(
        &mut session,
        Message::new("MODE", ["#papo", "+t"]),
        ResponseMatcher::Mode("#papo".into()),
        &[":ergo.test 482 papo #papo :You're not a channel operator"],
    );
    assert_unlabelled(
        &mut session,
        Message::new("TOPIC", ["#papo"]),
        ResponseMatcher::Topic("#papo".into()),
        &[":ergo.test 332 papo #papo :Requests in Rust"],
    );
    assert_unlabelled(
        &mut session,
        Message::new("WHO", ["#missing"]),
        ResponseMatcher::Who("#missing".into()),
        &["FAIL WHO INVALID_PARAMS #missing :Invalid mask"],
    );
    assert_unlabelled(
        &mut session,
        Message::new("LIST", Vec::<String>::new()),
        ResponseMatcher::List,
        &[":ergo.test 461 papo LIST :Not enough parameters"],
    );
}

#[test]
fn concurrent_unlabelled_whois_uses_targets_then_fifo_order() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    let alice = unlabelled_whois(&mut session, now, "alice");
    let bob = unlabelled_whois(&mut session, now, "bob");
    session.handle_line(now, b":ergo.test 401 papo bob :No such nick/channel");
    session.handle_line(now, b":ergo.test 401 papo alice :No such nick/channel");
    assert_eq!(
        (response_id(&mut session), response_id(&mut session)),
        (bob, alice)
    );

    let first = unlabelled_whois(&mut session, now, "missing");
    let second = unlabelled_whois(&mut session, now, "missing");
    for _ in 0..2 {
        session.handle_line(now, b":ergo.test 401 papo missing :No such nick/channel");
    }
    assert_eq!(
        (response_id(&mut session), response_id(&mut session)),
        (first, second)
    );
}

#[test]
fn whois_targets_and_casemapping_separate_concurrent_replies() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    let alice = unlabelled_whois(&mut session, now, "Alice[");
    let bob = unlabelled_whois(&mut session, now, "bob");
    for line in [
        ":ergo.test 311 papo bob ~bob users.example * :Bob Example",
        ":ergo.test 401 papo mallory :No such nick/channel",
        ":ergo.test 311 papo alice{ ~alice staff.example * :Alice Example",
        ":ergo.test 318 papo bob :End of /WHOIS list",
        ":ergo.test 318 papo alice{ :End of /WHOIS list",
    ] {
        session.handle_line(now, line.as_bytes());
    }

    let (responses, messages) = response_lines(&mut session);
    assert_eq!(
        responses[&bob]
            .iter()
            .map(|line| line.as_ref().param(1).unwrap())
            .collect::<Vec<_>>(),
        ["bob", "bob"]
    );
    assert_eq!(
        responses[&alice]
            .iter()
            .map(|line| line.as_ref().param(1).unwrap())
            .collect::<Vec<_>>(),
        ["alice{", "alice{"]
    );
    assert!(
        messages
            .iter()
            .any(|line| line.as_ref().param(1) == Some("mallory"))
    );
}

#[test]
fn who_rows_attach_only_to_the_named_concurrent_mask() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    let alice = session
        .request_with_matcher(
            now,
            Message::new("WHO", ["Alice["]),
            ResponseMatcher::Who("Alice[".into()),
        )
        .unwrap();
    let rust = session
        .request_with_matcher(
            now,
            Message::new("WHO", ["#rust"]),
            ResponseMatcher::Who("#rust".into()),
        )
        .unwrap();
    for line in [
        ":ergo.test 354 papo 0 #rust ~bob users.example bob H account :Bob Example",
        ":ergo.test 403 papo #unknown :No such channel",
        ":ergo.test 352 papo #papo ~alice staff.example ergo.test alice{ H@ :0 Alice Example",
        ":ergo.test 315 papo #rust :End of WHO list",
        ":ergo.test 315 papo alice{ :End of WHO list",
    ] {
        session.handle_line(now, line.as_bytes());
    }

    let (responses, messages) = response_lines(&mut session);
    assert!(responses[&rust][0].as_ref().command_is("354"));
    assert!(responses[&alice][0].as_ref().command_is("352"));
    assert_eq!(responses[&rust].len(), 2);
    assert_eq!(responses[&alice].len(), 2);
    assert!(
        messages
            .iter()
            .any(|line| line.as_ref().param(1) == Some("#unknown"))
    );
}

#[test]
fn join_sync_and_errors_use_the_named_concurrent_channel() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    let papo = session
        .request_with_matcher(
            now,
            Message::join("#Papo["),
            ResponseMatcher::Join("#Papo[".into()),
        )
        .unwrap();
    let invite = session
        .request_with_matcher(
            now,
            Message::join("#invite"),
            ResponseMatcher::Join("#invite".into()),
        )
        .unwrap();
    for line in [
        ":alice!alice@staff.example JOIN #papo{ alice :Alice Example",
        ":papo!papo@localhost JOIN #papo{ * :Papo Client",
        ":ergo.test 471 papo #unknown :Cannot join channel (+l)",
        ":ergo.test 473 papo #invite :Cannot join channel (+i)",
        ":ergo.test 353 papo = #papo{ :@papo alice",
        ":ergo.test 366 papo #papo{ :End of /NAMES list",
    ] {
        session.handle_line(now, line.as_bytes());
    }

    let (responses, messages) = response_lines(&mut session);
    assert_eq!(responses[&invite].len(), 1);
    assert_eq!(responses[&invite][0].as_ref().param(1), Some("#invite"));
    assert_eq!(responses[&papo].len(), 2);
    assert!(responses[&papo][0].as_ref().command_is("JOIN"));
    assert!(responses[&papo][1].as_ref().command_is("366"));
    assert!(
        messages
            .iter()
            .any(|line| line.as_ref().param(1) == Some("#unknown"))
    );
}

#[test]
fn part_echoes_require_our_source_and_the_named_concurrent_channel() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    let papo = session
        .request_with_matcher(
            now,
            Message::new("PART", ["#Papo["]),
            ResponseMatcher::Part("#Papo[".into()),
        )
        .unwrap();
    let rust = session
        .request_with_matcher(
            now,
            Message::new("PART", ["#rust"]),
            ResponseMatcher::Part("#rust".into()),
        )
        .unwrap();
    for line in [
        ":alice!alice@staff.example PART #papo{ :Leaving",
        ":ergo.test 403 papo #unknown :No such channel",
        ":papo!papo@localhost PART #rust :Requested",
        ":papo!papo@localhost PART #papo{ :Requested",
    ] {
        session.handle_line(now, line.as_bytes());
    }

    let (responses, messages) = response_lines(&mut session);
    for (id, channel) in [(rust, "#rust"), (papo, "#papo{")] {
        assert_eq!(responses[&id].len(), 1);
        assert_eq!(responses[&id][0].as_ref().param(0), Some(channel));
        assert_eq!(
            responses[&id][0].as_ref().raw_source(),
            Some("papo!papo@localhost")
        );
    }
    assert!(
        messages
            .iter()
            .any(|line| line.as_ref().param(1) == Some("#unknown"))
    );
}

#[test]
fn mode_echoes_and_errors_use_the_named_concurrent_target() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    let papo = session
        .request_with_matcher(
            now,
            Message::new("MODE", ["#Papo[", "+t"]),
            ResponseMatcher::Mode("#Papo[".into()),
        )
        .unwrap();
    let rust = session
        .request_with_matcher(
            now,
            Message::new("MODE", ["#rust", "+m"]),
            ResponseMatcher::Mode("#rust".into()),
        )
        .unwrap();
    for line in [
        ":ergo.test 482 papo #unknown :You're not a channel operator",
        ":ergo.test 482 papo #rust :You're not a channel operator",
        ":alice!alice@staff.example MODE #unknown +m",
        ":papo!papo@localhost MODE #papo{ +t",
    ] {
        session.handle_line(now, line.as_bytes());
    }

    let (responses, messages) = response_lines(&mut session);
    assert_eq!(responses[&rust][0].as_ref().param(1), Some("#rust"));
    assert_eq!(responses[&papo][0].as_ref().param(0), Some("#papo{"));
    assert!(
        messages
            .iter()
            .any(|line| line.as_ref().param(1) == Some("#unknown"))
    );
}

#[test]
fn topic_replies_use_the_named_concurrent_channel() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    let papo = session
        .request_with_matcher(
            now,
            Message::new("TOPIC", ["#Papo["]),
            ResponseMatcher::Topic("#Papo[".into()),
        )
        .unwrap();
    let rust = session
        .request_with_matcher(
            now,
            Message::new("TOPIC", ["#rust"]),
            ResponseMatcher::Topic("#rust".into()),
        )
        .unwrap();
    for line in [
        ":ergo.test 332 papo #rust :The Rust Programming Language",
        ":ergo.test 403 papo #unknown :No such channel",
        ":alice!alice@staff.example TOPIC #unknown :Unrelated topic",
        ":alice!alice@staff.example TOPIC #papo{ :Requests in Rust",
    ] {
        session.handle_line(now, line.as_bytes());
    }

    let (responses, messages) = response_lines(&mut session);
    assert_eq!(responses[&rust][0].as_ref().param(1), Some("#rust"));
    assert_eq!(responses[&papo][0].as_ref().param(0), Some("#papo{"));
    assert!(
        messages
            .iter()
            .any(|line| line.as_ref().param(1) == Some("#unknown"))
    );
}

#[test]
fn list_replies_use_fifo_without_swallowing_unrelated_errors() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    let first = session
        .request_with_matcher(
            now,
            Message::new("LIST", Vec::<String>::new()),
            ResponseMatcher::List,
        )
        .unwrap();
    let second = session
        .request_with_matcher(now, Message::new("LIST", ["#rust"]), ResponseMatcher::List)
        .unwrap();
    for line in [
        ":ergo.test 401 papo alice :No such nick/channel",
        ":ergo.test 322 papo #papo 42 :Ergonomic IRC",
        ":ergo.test 323 papo :End of /LIST",
        ":ergo.test 322 papo #rust 17 :The Rust Programming Language",
        ":ergo.test 323 papo :End of /LIST",
    ] {
        session.handle_line(now, line.as_bytes());
    }

    let (responses, messages) = response_lines(&mut session);
    assert_eq!(responses[&first][0].as_ref().param(1), Some("#papo"));
    assert_eq!(responses[&second][0].as_ref().param(1), Some("#rust"));
    assert!(messages.iter().any(|line| line.as_ref().command_is("401")));
}

#[test]
fn standard_failures_require_fail_kind_and_matching_command() {
    let now = Instant::now();
    let cases = [
        (
            "WHOIS",
            Message::new("WHOIS", ["alice"]),
            ResponseMatcher::Whois("alice".into()),
        ),
        (
            "WHO",
            Message::new("WHO", ["#papo"]),
            ResponseMatcher::Who("#papo".into()),
        ),
        (
            "JOIN",
            Message::join("#papo"),
            ResponseMatcher::Join("#papo".into()),
        ),
        (
            "PART",
            Message::new("PART", ["#papo"]),
            ResponseMatcher::Part("#papo".into()),
        ),
        (
            "MODE",
            Message::new("MODE", ["#papo", "+t"]),
            ResponseMatcher::Mode("#papo".into()),
        ),
        (
            "TOPIC",
            Message::new("TOPIC", ["#papo"]),
            ResponseMatcher::Topic("#papo".into()),
        ),
        (
            "LIST",
            Message::new("LIST", Vec::<String>::new()),
            ResponseMatcher::List,
        ),
    ];

    for (command, message, matcher) in cases {
        let mut session = Session::new(Config::default());
        let id = session.request_with_matcher(now, message, matcher).unwrap();
        session.handle_line(
            now,
            format!("WARN {command} INVALID_PARAMS :Not a failure").as_bytes(),
        );
        session.handle_line(now, b"FAIL CHATHISTORY INVALID_PARAMS :Different command");
        assert!(
            std::iter::from_fn(|| session.poll_event())
                .all(|event| !matches!(event, Event::Response(_)))
        );

        session.handle_line(
            now,
            format!(
                "FAIL {} INVALID_PARAMS :Rejected",
                command.to_ascii_lowercase()
            )
            .as_bytes(),
        );
        assert_eq!(response_id(&mut session), id);
    }
}

#[test]
fn unlabelled_requests_time_out_abort_and_obey_the_literal_limit() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    let first = session
        .request_with_matcher(
            now,
            Message::new("WHOIS", ["alice"]),
            ResponseMatcher::Whois("alice".into()),
        )
        .unwrap();
    assert_eq!(
        session.poll_timeout(),
        Some(now + std::time::Duration::from_secs(30))
    );
    session.handle_timeout(now + std::time::Duration::from_secs(30));
    assert!(matches!(session.poll_event(), Some(Event::RequestTimedOut { id }) if id == first));

    for index in 0..256 {
        session
            .request_with_matcher(
                now,
                Message::new("WHOIS", [format!("user{index}")]),
                ResponseMatcher::Whois(format!("user{index}")),
            )
            .unwrap();
    }
    assert_eq!(
        session.request_with_matcher(
            now,
            Message::new("WHOIS", ["overflow"]),
            ResponseMatcher::Whois("overflow".into())
        ),
        Err(RequestError::PendingLimit)
    );
    session.handle_disconnected(now);
    assert_eq!(
        std::iter::from_fn(|| session.poll_event())
            .filter(|event| matches!(event, Event::RequestAborted { .. }))
            .count(),
        256
    );
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

    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("expected echo response")
    };
    assert_eq!(response.id, echo);
    assert_eq!(response.lines.len(), 1);
    assert_eq!(
        response.lines[0].as_ref().tag("msgid").as_deref(),
        Some("RKi9Cw")
    );
    assert_eq!(response.sent(), None);
    assert!(matches!(
        session.poll_event(),
        Some(Event::Response(response)) if response.id == mode && response.lines.is_empty()
    ));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn exposes_the_msgid_and_time_of_a_labelled_echo() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :echo-message labeled-response");
    session.handle_line(now, b"CAP * ACK :echo-message labeled-response");
    while session.poll_transmit().is_some() {}
    let id = request(
        &mut session,
        now,
        Message::privmsg("#papo", "hello from papo"),
    );
    session.handle_line(
        now,
        b"@label=0;msgid=RKi9Cw;time=2026-10-15T12:00:01.000Z :papo!~papo@localhost PRIVMSG #papo :hello from papo",
    );

    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("expected response")
    };
    assert_eq!(response.id, id);
    let sent = response.sent().expect("echo identity");
    assert_eq!(sent.msgid, "RKi9Cw");
    assert_eq!(sent.time.as_deref(), Some("2026-10-15T12:00:01.000Z"));
}

#[test]
fn does_not_report_sent_info_for_a_non_message_request() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :echo-message labeled-response");
    session.handle_line(now, b"CAP * ACK :echo-message labeled-response");
    while session.poll_transmit().is_some() {}
    request(&mut session, now, Message::new("WHOIS", ["alice"]));
    session.handle_line(
        now,
        b"@label=0;msgid=RKi9Cw :ergo.test 318 papo alice :End of /WHOIS list",
    );

    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("expected response")
    };
    assert_eq!(response.sent(), None);
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
fn pending_non_history_batch_dispatches_protocol_state() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = request(&mut session, now, Message::join("#papo"));
    session.handle_line(now, b"@label=0 :ergo.test BATCH +join-42 labeled-response");
    session.handle_line(
        now,
        b"@batch=join-42 :papo!~papo@localhost JOIN #papo papo :Papo Bot",
    );
    session.handle_line(now, b":ergo.test BATCH -join-42");

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
fn unclaimed_history_response_releases_its_messages() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    session.handle_line(
        now,
        b"@label=expired :ergo.test BATCH +reply-42 labeled-response",
    );
    session.handle_line(
        now,
        b"@batch=reply-42 :ergo.test BATCH +history-42 chathistory #papo",
    );
    session.handle_line(
        now,
        b"@batch=history-42;time=2026-10-15T12:00:01.000Z;msgid=RKi9Cw :alice!~a@localhost PRIVMSG #papo :earlier",
    );
    session.handle_line(now, b"@batch=reply-42 :ergo.test BATCH -history-42");
    session.handle_line(now, b":ergo.test BATCH -reply-42");

    assert!(matches!(
        session.poll_event(),
        Some(Event::History { target, messages, .. }) if target == "#papo" && messages.len() == 1
    ));
    assert!(matches!(
        session.poll_event(),
        Some(Event::Message(message))
            if message.as_ref().command_is("PRIVMSG")
                && message.batch_type() == Some("labeled-response")
    ));
    assert_eq!(session.poll_event(), None);
}

#[test]
fn unclaimed_response_batches_preserve_their_distinct_types() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    for (reference, label, batch_type, text) in [
        ("sync-42", "expired-a", "vendor/sync", "sync complete"),
        (
            "reply-42",
            "expired-b",
            "labeled-response",
            "request complete",
        ),
    ] {
        session.handle_line(
            now,
            format!("@label={label} :ergo.test BATCH +{reference} {batch_type}").as_bytes(),
        );
        session.handle_line(
            now,
            format!("@batch={reference} :ergo.test NOTICE papo :{text}").as_bytes(),
        );
        session.handle_line(now, format!(":ergo.test BATCH -{reference}").as_bytes());
        assert!(matches!(
            session.poll_event(),
            Some(Event::Message(message)) if message.batch_type() == Some(batch_type)
        ));
    }
    assert_eq!(session.poll_event(), None);
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

#[test]
fn reports_only_well_formed_unlabelled_standard_replies() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_line(
        now,
        b":ergo.test FAIL CHATHISTORY INVALID_PARAMS #papo :Invalid parameters",
    );
    assert!(matches!(
        session.poll_event(),
        Some(Event::StandardReply(reply))
            if reply.kind == papo_proto::StandardReplyKind::Fail
                && reply.command == "CHATHISTORY"
                && reply.code == "INVALID_PARAMS"
                && reply.context == ["#papo"]
                && reply.description == "Invalid parameters"
    ));

    session.handle_line(now, b":ergo.test WARN CHATHISTORY MISSING_DESCRIPTION");
    assert!(
        matches!(session.poll_event(), Some(Event::Message(message)) if message.as_ref().command_is("WARN"))
    );
}
