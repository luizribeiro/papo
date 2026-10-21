//! Transcript coverage for labelled request correlation.

use std::{collections::BTreeMap, time::Instant};

use papo_core::{
    Config, Conversation, Event, RateLimit, Refusal, RequestError, RequestId, ResponseMatcher,
    ServerError, Session, TextKind, WhoOnJoin,
};
use papo_proto::{Command, Line, LineRef, Numeric};

fn labelled_session(now: Instant) -> Session {
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :labeled-response");
    session.handle_line(now, b"CAP * ACK :labeled-response");
    while session.poll_transmit().is_some() {}
    session
}

fn echo_session(now: Instant) -> Session {
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :echo-message labeled-response");
    session.handle_line(now, b"CAP * ACK :echo-message labeled-response");
    while session.poll_transmit().is_some() {}
    session
}

fn request(session: &mut Session, now: Instant, message: Line) -> RequestId {
    session.request(now, message).unwrap()
}

#[test]
fn matches_an_unlabelled_whois_through_its_end_numeric() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    let id = session
        .request_with_matcher(
            now,
            Line::new("WHOIS", ["Alice["]),
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
fn parses_whois_channels_with_advertised_membership_prefixes() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    session.handle_line(
        now,
        b":irc.unreal.example 005 papo PREFIX=(qaohv)~&@%+ CHANTYPES=#&+! :are supported",
    );
    session
        .request_with_matcher(
            now,
            Line::new("WHOIS", ["alice"]),
            ResponseMatcher::Whois("alice".into()),
        )
        .unwrap();
    session.handle_line(
        now,
        b":irc.unreal.example 319 papo alice :~&@#staff +#papo !support ~&@",
    );
    session.handle_line(
        now,
        b":irc.unreal.example 318 papo alice :End of /WHOIS list",
    );

    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("expected a WHOIS response")
    };
    assert_eq!(
        response.whois_channels,
        [
            papo_core::WhoisChannel {
                name: "#staff".into(),
                prefixes: vec!['~', '&', '@'],
            },
            papo_core::WhoisChannel {
                name: "#papo".into(),
                prefixes: vec!['+'],
            },
            papo_core::WhoisChannel {
                name: "!support".into(),
                prefixes: Vec::new(),
            },
        ]
    );
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
            Line::join("#Papo["),
            ResponseMatcher::Join("#Papo[".into()),
        )
        .unwrap();
    for line in [
        ":papo!papo@localhost JOIN #papo{ * :Papo Client",
        ":ergo.test 353 papo = #papo{ :@papo alice",
        ":ergo.test 366 papo #papo{ :End of /NAMES list",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    assert!(
        std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::Response(response) if response.id == id))
    );
}

#[test]
fn unlabelled_mode_waits_for_its_matching_ping_fence() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    let id = session
        .request_with_matcher(
            now,
            Line::new("MODE", ["#papo", "+t"]),
            ResponseMatcher::Mode("#papo".into()),
        )
        .unwrap();
    assert_eq!(session.poll_transmit().unwrap(), b"MODE #papo +t\r\n");
    assert_eq!(session.poll_transmit().unwrap(), b"PING :papo-mode-0\r\n");

    session.handle_line(now, b":papo!papo@localhost MODE #papo +t");
    assert!(
        !std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::Response(_)))
    );
    session.handle_line(now, b":ergo.test PONG ergo.test :wrong-token");
    assert!(
        !std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::Response(_)))
    );
    session.handle_line(now, b":ergo.test PONG ergo.test :papo-mode-0");

    assert!(
        std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::Response(response) if response.id == id))
    );
}

#[test]
fn unlabelled_mode_keeps_applied_changes_and_partial_refusals() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    let id = session
        .request_with_matcher(
            now,
            Line::new(
                "MODE",
                ["#papo", "+bb", "alice!*@*", "spambot!*@gateway.example"],
            ),
            ResponseMatcher::Mode("#papo".into()),
        )
        .unwrap();
    assert_eq!(
        session.poll_transmit().unwrap(),
        b"MODE #papo +bb alice!*@* spambot!*@gateway.example\r\n"
    );
    assert_eq!(session.poll_transmit().unwrap(), b"PING :papo-mode-0\r\n");

    session.handle_line(now, b":papo!~papo@localhost MODE #papo +b alice!*@*");
    session.handle_line(
        now,
        b":ergo.test 478 papo #papo b :Channel ban list is full",
    );
    session.handle_line(now, b":ergo.test PONG ergo.test :papo-mode-0");

    let response = std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::Response(response) if response.id == id => Some(response),
            _ => None,
        })
        .unwrap();
    assert_eq!(response.lines.len(), 3);
    assert_eq!(response.refusal(), Some(Refusal::BanListFull));
    assert_eq!(response.lines[0].as_ref().param(2), Some("alice!*@*"));
    assert_eq!(
        response.lines[1].as_ref().command(),
        Command::Numeric(Numeric::new(478).unwrap())
    );
}

#[test]
fn ban_confirmation_reaches_the_mode_fence_after_echo_or_duplicate() {
    for confirmation in [
        ":papo!~papo@localhost MODE #papo +b alice!*@*",
        ":irc.inspircd.test 697 papo #papo alice!*@* b :Entry is already in the list",
    ] {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        let id = session
            .request_with_matcher(
                now,
                Line::new("MODE", ["#papo", "+b", "alice"]),
                ResponseMatcher::Mode("#papo".into()),
            )
            .unwrap();
        assert_eq!(session.poll_transmit().unwrap(), b"MODE #papo +b alice\r\n");
        assert_eq!(session.poll_transmit().unwrap(), b"PING :papo-mode-0\r\n");

        session.handle_line(now, confirmation.as_bytes());
        assert!(
            !std::iter::from_fn(|| session.poll_event())
                .any(|event| matches!(event, Event::Response(_)))
        );
        session.handle_line(now, b":ergo.test PONG ergo.test :papo-mode-0");

        let response = std::iter::from_fn(|| session.poll_event())
            .find_map(|event| match event {
                Event::Response(response) if response.id == id => Some(response),
                _ => None,
            })
            .unwrap();
        assert_eq!(response.lines[0].as_ref().param(2), Some("alice!*@*"));
        assert_eq!(response.lines.len(), 2);
    }
}

fn assert_unlabelled(
    session: &mut Session,
    message: Line,
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

fn assert_unlabelled_refusal(
    message: Line,
    matcher: ResponseMatcher,
    line: &str,
    expected: Refusal,
) {
    let now = Instant::now();
    let mode = matches!(matcher, ResponseMatcher::Mode(_));
    let mut session = Session::new(Config::default().nick("papo"));
    let id = session.request_with_matcher(now, message, matcher).unwrap();
    session.handle_line(now, line.as_bytes());
    if mode {
        session.handle_line(now, b":ergo.test PONG ergo.test :papo-mode-0");
    }
    assert!(matches!(
        session.poll_event(),
        Some(Event::Response(response))
            if response.id == id && response.refusal() == Some(expected)
    ));
}

fn response_id(session: &mut Session) -> RequestId {
    std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::Response(response) => Some(response.id),
            _ => None,
        })
        .unwrap()
}

fn assert_no_response(session: &mut Session) {
    assert!(
        std::iter::from_fn(|| session.poll_event())
            .all(|event| !matches!(event, Event::Response(_)))
    );
}

fn response_lines(session: &mut Session) -> (BTreeMap<RequestId, Vec<Line>>, Vec<Line>) {
    let mut responses = BTreeMap::new();
    let mut messages = Vec::new();
    while let Some(event) = session.poll_event() {
        match event {
            Event::Response(response) => {
                responses.insert(response.id, response.lines);
            }
            Event::Line(line) => messages.push(line.into_line()),
            _ => {}
        }
    }
    (responses, messages)
}

fn unlabelled_whois(session: &mut Session, now: Instant, nick: &str) -> RequestId {
    session
        .request_with_matcher(
            now,
            Line::new("WHOIS", [nick]),
            ResponseMatcher::Whois(nick.into()),
        )
        .unwrap()
}

#[test]
fn matches_other_unlabelled_request_boundaries() {
    let mut session = Session::new(Config::default().nick("papo"));
    assert_unlabelled(
        &mut session,
        Line::new("WHO", ["#papo"]),
        ResponseMatcher::Who("#papo".into()),
        &[
            ":ergo.test 352 papo #papo ~alice host ergo.test alice H@ :0 Alice Example",
            ":ergo.test 315 papo #papo :End of WHO list",
        ],
    );
    assert_unlabelled(
        &mut session,
        Line::new("LIST", Vec::<String>::new()),
        ResponseMatcher::List,
        &[
            ":ergo.test 321 papo Channel :Users Name",
            ":ergo.test 322 papo #papo 42 :Ergonomic IRC",
            ":ergo.test 323 papo :End of /LIST",
        ],
    );
    assert_unlabelled(
        &mut session,
        Line::new("PART", ["#papo"]),
        ResponseMatcher::Part("#papo".into()),
        &[":papo!papo@localhost PART #papo :Leaving"],
    );
    assert_unlabelled(
        &mut session,
        Line::new("MODE", ["#papo", "+t"]),
        ResponseMatcher::Mode("#papo".into()),
        &[
            ":ergo.test 482 papo #papo :You're not a channel operator",
            ":ergo.test PONG ergo.test :papo-mode-3",
        ],
    );
    assert_unlabelled(
        &mut session,
        Line::new("TOPIC", ["#papo"]),
        ResponseMatcher::Topic("#papo".into()),
        &[":ergo.test 332 papo #papo :Requests in Rust"],
    );
    assert_unlabelled(
        &mut session,
        Line::new("WHO", ["#missing"]),
        ResponseMatcher::Who("#missing".into()),
        &["FAIL WHO INVALID_PARAMS #missing :Invalid mask"],
    );
    assert_unlabelled(
        &mut session,
        Line::new("LIST", Vec::<String>::new()),
        ResponseMatcher::List,
        &[":ergo.test 461 papo LIST :Not enough parameters"],
    );
}

#[test]
fn matches_unlabelled_membership_and_nick_actions() {
    let mut session = Session::new(Config::default().nick("papo"));
    assert_unlabelled(
        &mut session,
        Line::new("KICK", ["#papo", "spambot", "flooding"]),
        ResponseMatcher::Kick {
            channel: "#papo".into(),
            nick: "spambot".into(),
        },
        &[":alice!~a@localhost KICK #papo spambot :flooding"],
    );
    assert_unlabelled(
        &mut session,
        Line::new("INVITE", ["carol", "#papo"]),
        ResponseMatcher::Invite {
            channel: "#papo".into(),
            nick: "carol".into(),
        },
        &[":ergo.test 341 papo carol #papo"],
    );
    assert_unlabelled(
        &mut session,
        Line::new("NICK", ["papo-away"]),
        ResponseMatcher::Nick("papo-away".into()),
        &[":papo!papo@localhost NICK :papo-away"],
    );
}

#[test]
fn typed_action_refusals_complete_unlabelled_requests() {
    let cases = [
        (
            Line::join("#invite"),
            ResponseMatcher::Join("#invite".into()),
            ":ergo.test 473 papo #invite :Cannot join channel (+i)",
            Refusal::InviteOnlyChannel,
        ),
        (
            Line::new("KICK", ["#papo", "carol"]),
            ResponseMatcher::Kick {
                channel: "#papo".into(),
                nick: "carol".into(),
            },
            ":irc.solanum.test 441 papo carol #papo :They aren't on that channel",
            Refusal::UserNotInChannel,
        ),
        (
            Line::new("MODE", ["#papo", "+z"]),
            ResponseMatcher::Mode("#papo".into()),
            ":irc.solanum.test 472 papo z :is unknown mode char to me",
            Refusal::UnknownMode,
        ),
        (
            Line::new("NICK", ["42papo"]),
            ResponseMatcher::Nick("42papo".into()),
            ":ergo.test 432 papo 42papo :Erroneous nickname",
            Refusal::ErroneousNickname,
        ),
    ];
    for (message, matcher, line, refusal) in cases {
        assert_unlabelled_refusal(message, matcher, line, refusal);
    }
}

#[test]
fn matches_unlabelled_own_user_actions() {
    let mut session = Session::new(Config::default().nick("papo"));
    assert_unlabelled(
        &mut session,
        Line::new("AWAY", ["reviewing patches"]),
        ResponseMatcher::Away(true),
        &[":ergo.test 306 papo :You have been marked as being away"],
    );
    assert_unlabelled(
        &mut session,
        Line::new("AWAY", Vec::<String>::new()),
        ResponseMatcher::Away(false),
        &[":ergo.test 305 papo :You are no longer marked as being away"],
    );
    assert_unlabelled(
        &mut session,
        Line::new("SETNAME", ["Papo Test Client"]),
        ResponseMatcher::SetName("Papo Test Client".into()),
        &[":papo!papo@localhost SETNAME :Papo Test Client"],
    );
    assert_unlabelled(
        &mut session,
        Line::new("OPER", ["papo", "correct horse battery staple"]),
        ResponseMatcher::Oper,
        &[":ergo.test 381 papo :You are now an IRC operator"],
    );
    assert_unlabelled(
        &mut session,
        Line::new("PING", ["papo-lag-42"]),
        ResponseMatcher::Ping("papo-lag-42".into()),
        &[":ergo.test PONG ergo.test :papo-lag-42"],
    );
}

#[test]
fn own_user_matchers_require_their_exact_success() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    let id = session
        .request_with_matcher(
            now,
            Line::new("PING", ["papo-lag-42"]),
            ResponseMatcher::Ping("papo-lag-42".into()),
        )
        .unwrap();
    session.handle_line(now, b":ergo.test PONG ergo.test :papo-lag-41");
    assert!(
        !std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::Response(_)))
    );
    session.handle_line(now, b":ergo.test PONG ergo.test :papo-lag-42");
    assert_eq!(response_id(&mut session), id);
}

#[test]
fn setname_matches_after_a_nickname_change() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    let nick = session
        .request_with_matcher(
            now,
            Line::new("NICK", ["paporev"]),
            ResponseMatcher::Nick("paporev".into()),
        )
        .unwrap();
    session.handle_line(now, b":papo!papo@localhost NICK :paporev");
    assert_eq!(response_id(&mut session), nick);

    let realname = session
        .request_with_matcher(
            now,
            Line::new("SETNAME", ["Papo Review Client"]),
            ResponseMatcher::SetName("Papo Review Client".into()),
        )
        .unwrap();
    session.handle_line(now, b":paporev!papo@localhost SETNAME :Papo Review Client");
    assert_eq!(response_id(&mut session), realname);
}

#[test]
fn membership_matchers_ignore_unrelated_successes_and_capture_errors() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    let id = session
        .request_with_matcher(
            now,
            Line::new("INVITE", ["carol", "#papo"]),
            ResponseMatcher::Invite {
                channel: "#papo".into(),
                nick: "carol".into(),
            },
        )
        .unwrap();
    session.handle_line(now, b":ergo.test 341 papo dan #rust");
    assert!(
        !std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::Response(_)))
    );
    session.handle_line(now, b":ergo.test 401 papo carol :No such nick/channel");
    assert_eq!(response_id(&mut session), id);
}

#[test]
fn action_matchers_require_every_identity_field() {
    let now = Instant::now();

    let mut kick = Session::new(Config::default().nick("papo"));
    let kick_id = kick
        .request_with_matcher(
            now,
            Line::new("KICK", ["#papo", "spambot"]),
            ResponseMatcher::Kick {
                channel: "#papo".into(),
                nick: "spambot".into(),
            },
        )
        .unwrap();
    for line in [
        ":alice!alice@staff.example KICK #papo alice :Requested",
        ":alice!alice@staff.example KICK #rust spambot :Requested",
        ":ergo.test 332 papo #papo :An unrelated topic",
        ":irc.solanum.test 441 papo spambot #rust :They aren't on that channel",
    ] {
        kick.handle_line(now, line.as_bytes());
        assert_no_response(&mut kick);
    }
    kick.handle_line(
        now,
        b":irc.solanum.test 441 papo spambot #papo :They aren't on that channel",
    );
    assert_eq!(response_id(&mut kick), kick_id);

    let mut invite = Session::new(Config::default().nick("papo"));
    let invite_id = invite
        .request_with_matcher(
            now,
            Line::new("INVITE", ["carol", "#papo"]),
            ResponseMatcher::Invite {
                channel: "#papo".into(),
                nick: "carol".into(),
            },
        )
        .unwrap();
    for line in [
        ":ergo.test 341 papo carol #rust",
        ":ergo.test 332 papo carol :An unrelated topic",
    ] {
        invite.handle_line(now, line.as_bytes());
        assert_no_response(&mut invite);
    }
    invite.handle_line(now, b":ergo.test 401 papo carol :No such nick/channel");
    assert_eq!(response_id(&mut invite), invite_id);

    let mut nick = Session::new(Config::default().nick("papo"));
    let nick_id = nick
        .request_with_matcher(
            now,
            Line::new("NICK", ["papo-away"]),
            ResponseMatcher::Nick("papo-away".into()),
        )
        .unwrap();
    for line in [
        ":alice!alice@staff.example NICK :papo-away",
        ":papo!papo@localhost NICK :papo-busy",
        ":ergo.test 332 papo papo-away :An unrelated topic",
    ] {
        nick.handle_line(now, line.as_bytes());
        assert_no_response(&mut nick);
    }
    nick.handle_line(
        now,
        b":ergo.test 433 papo papo-away :Nickname is already in use",
    );
    assert_eq!(response_id(&mut nick), nick_id);

    let mut setname = Session::new(Config::default().nick("papo"));
    let setname_id = setname
        .request_with_matcher(
            now,
            Line::new("SETNAME", ["Papo Test Client"]),
            ResponseMatcher::SetName("Papo Test Client".into()),
        )
        .unwrap();
    for line in [
        ":alice!alice@staff.example SETNAME :Papo Test Client",
        ":papo!papo@localhost SETNAME :Different Name",
    ] {
        setname.handle_line(now, line.as_bytes());
        assert_no_response(&mut setname);
    }
    setname.handle_line(now, b":papo!papo@localhost SETNAME :Papo Test Client");
    assert_eq!(response_id(&mut setname), setname_id);
}

#[test]
fn kick_refusals_resolve_only_the_named_concurrent_request() {
    let now = Instant::now();
    let cases = [
        (
            ":ergo.test 401 papo alice :No such nick/channel",
            ":ergo.test 401 papo spambot :No such nick/channel",
        ),
        (
            ":irc.solanum.test 403 papo #rust :No such channel",
            ":irc.solanum.test 403 papo #papo :No such channel",
        ),
        (
            ":irc.unreal.test 442 papo #rust :You're not on that channel",
            ":irc.unreal.test 442 papo #papo :You're not on that channel",
        ),
        (
            ":ergo.test 482 papo #rust :You're not a channel operator",
            ":ergo.test 482 papo #papo :You're not a channel operator",
        ),
    ];

    for (unrelated, refusal) in cases {
        let mut session = Session::new(Config::default().nick("papo"));
        let kick = session
            .request_with_matcher(
                now,
                Line::new("KICK", ["#papo", "spambot"]),
                ResponseMatcher::Kick {
                    channel: "#papo".into(),
                    nick: "spambot".into(),
                },
            )
            .unwrap();
        let mode = session
            .request_with_matcher(
                now,
                Line::new("MODE", ["#review", "+m"]),
                ResponseMatcher::Mode("#review".into()),
            )
            .unwrap();

        session.handle_line(now, unrelated.as_bytes());
        assert_no_response(&mut session);
        session.handle_line(now, refusal.as_bytes());
        assert_eq!(response_id(&mut session), kick);

        session.handle_line(now, b":papo!papo@localhost MODE #review +m");
        assert_no_response(&mut session);
        session.handle_line(now, b":ergo.test PONG ergo.test :papo-mode-1");
        assert_eq!(response_id(&mut session), mode);
    }
}

#[test]
fn invite_refusals_resolve_only_the_named_concurrent_request() {
    let now = Instant::now();
    let cases: [(&[&str], &str); 4] = [
        (
            &[":irc.solanum.test 403 papo #rust :No such channel"],
            ":irc.solanum.test 403 papo #papo :No such channel",
        ),
        (
            &[":irc.unreal.test 442 papo #rust :You're not on that channel"],
            ":irc.unreal.test 442 papo #papo :You're not on that channel",
        ),
        (
            &[":ergo.test 482 papo #rust :You're not a channel operator"],
            ":ergo.test 482 papo #papo :You're not a channel operator",
        ),
        (
            &[
                ":irc.unreal.test 443 papo carol #rust :is already on channel",
                ":irc.unreal.test 443 papo alice #papo :is already on channel",
            ],
            ":irc.unreal.test 443 papo carol #papo :is already on channel",
        ),
    ];

    for (unrelated, refusal) in cases {
        let mut session = Session::new(Config::default().nick("papo"));
        let invite = session
            .request_with_matcher(
                now,
                Line::new("INVITE", ["carol", "#papo"]),
                ResponseMatcher::Invite {
                    channel: "#papo".into(),
                    nick: "carol".into(),
                },
            )
            .unwrap();
        let mode = session
            .request_with_matcher(
                now,
                Line::new("MODE", ["#review", "+m"]),
                ResponseMatcher::Mode("#review".into()),
            )
            .unwrap();

        for line in unrelated {
            session.handle_line(now, line.as_bytes());
            assert_no_response(&mut session);
        }
        session.handle_line(now, refusal.as_bytes());
        assert_eq!(response_id(&mut session), invite);

        session.handle_line(now, b":papo!papo@localhost MODE #review +m");
        assert_no_response(&mut session);
        session.handle_line(now, b":ergo.test PONG ergo.test :papo-mode-1");
        assert_eq!(response_id(&mut session), mode);
    }
}

#[test]
fn part_refusals_resolve_only_the_named_concurrent_request() {
    let now = Instant::now();
    for (unrelated, refusal) in [
        (
            ":irc.solanum.test 403 papo #rust :No such channel",
            ":irc.solanum.test 403 papo #papo :No such channel",
        ),
        (
            ":irc.unreal.test 442 papo #rust :You're not on that channel",
            ":irc.unreal.test 442 papo #papo :You're not on that channel",
        ),
    ] {
        let mut session = Session::new(Config::default().nick("papo"));
        let part = session
            .request_with_matcher(
                now,
                Line::new("PART", ["#papo"]),
                ResponseMatcher::Part("#papo".into()),
            )
            .unwrap();
        let mode = session
            .request_with_matcher(
                now,
                Line::new("MODE", ["#review", "+m"]),
                ResponseMatcher::Mode("#review".into()),
            )
            .unwrap();

        session.handle_line(now, unrelated.as_bytes());
        assert_no_response(&mut session);
        session.handle_line(now, refusal.as_bytes());
        assert_eq!(response_id(&mut session), part);

        session.handle_line(now, b":papo!papo@localhost MODE #review +m");
        assert_no_response(&mut session);
        session.handle_line(now, b":ergo.test PONG ergo.test :papo-mode-1");
        assert_eq!(response_id(&mut session), mode);
    }
}

#[test]
fn non_error_numeric_named_like_a_command_does_not_claim_its_request() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    let ping = session
        .request_with_matcher(
            now,
            Line::new("PING", ["routing-token"]),
            ResponseMatcher::Ping("routing-token".into()),
        )
        .unwrap();
    let invite = session
        .request_with_matcher(
            now,
            Line::new("INVITE", ["PING", "#papo"]),
            ResponseMatcher::Invite {
                channel: "#papo".into(),
                nick: "PING".into(),
            },
        )
        .unwrap();

    session.handle_line(now, b":ergo.test 341 papo PING #papo");
    assert_eq!(response_id(&mut session), invite);
    session.handle_line(now, b":ergo.test PONG ergo.test :routing-token");
    assert_eq!(response_id(&mut session), ping);
}

#[test]
fn no_origin_refusal_resolves_ping_and_leaves_invite_pending() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    let ping = session
        .request_with_matcher(
            now,
            Line::new("PING", Vec::<String>::new()),
            ResponseMatcher::Ping("missing-token".into()),
        )
        .unwrap();
    let invite = session
        .request_with_matcher(
            now,
            Line::new("INVITE", ["carol", "#papo"]),
            ResponseMatcher::Invite {
                channel: "#papo".into(),
                nick: "carol".into(),
            },
        )
        .unwrap();

    session.handle_line(now, b":irc.solanum.test 409 papo :No origin specified");
    assert_eq!(response_id(&mut session), ping);
    session.handle_line(now, b":ergo.test 341 papo carol #papo");
    assert_eq!(response_id(&mut session), invite);
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
            Line::new("WHO", ["Alice["]),
            ResponseMatcher::Who("Alice[".into()),
        )
        .unwrap();
    let rust = session
        .request_with_matcher(
            now,
            Line::new("WHO", ["#rust"]),
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

fn who_request_outcome(
    session: &mut Session,
    now: Instant,
    message: Line,
    mask: &str,
    row: &str,
) -> (bool, bool) {
    let id = session
        .request_with_matcher(now, message, ResponseMatcher::Who(mask.into()))
        .unwrap();
    session.handle_line(now, row.as_bytes());
    session.handle_line(
        now,
        format!(":ergo.test 315 papo {mask} :End of WHO list").as_bytes(),
    );

    let mut refreshed = false;
    let mut responded = false;
    while let Some(event) = session.poll_event() {
        match event {
            Event::MembersRefreshed { .. } => refreshed = true,
            Event::Response(response) if response.id == id => {
                assert_eq!(response.lines.len(), 2);
                responded = true;
            }
            _ => {}
        }
    }
    (refreshed, responded)
}

#[test]
fn manual_who_tracks_only_a_real_request_for_a_joined_channel() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited())
            .who_on_join(WhoOnJoin::Never),
    );
    for line in [
        ":papo!papo@localhost JOIN #papo",
        ":ergo.test 353 papo = #papo :@papo",
        ":ergo.test 366 papo #papo :End of NAMES list",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    while session.poll_event().is_some() {}
    while session.poll_transmit().is_some() {}

    let (refreshed, responded) = who_request_outcome(
        &mut session,
        now,
        Line::new("WHO", ["#papo"]),
        "#papo",
        ":ergo.test 352 papo #papo ~alice staff.example ergo.test alice H@ :0 Alice Example",
    );
    assert_eq!((refreshed, responded), (true, true));
    assert_eq!(
        session.state().user("alice").unwrap().realname(),
        Some("Alice Example")
    );

    let (refreshed, responded) = who_request_outcome(
        &mut session,
        now,
        Line::new("WHO", ["#rust"]),
        "#rust",
        ":ergo.test 352 papo #rust ~bob users.example ergo.test bob H :0 Bob Example",
    );
    assert!(!refreshed && responded);
    assert!(session.state().user("bob").is_none());

    let (refreshed, responded) = who_request_outcome(
        &mut session,
        now,
        Line::new("WHO", ["carol"]),
        "carol",
        ":ergo.test 352 papo #papo ~carol users.example ergo.test carol H :0 Carol Example",
    );
    assert!(!refreshed && responded);
    assert!(session.state().user("carol").is_none());

    let (refreshed, responded) = who_request_outcome(
        &mut session,
        now,
        Line::new("WHOIS", ["#papo"]),
        "#papo",
        ":ergo.test 352 papo #papo ~mallory users.example ergo.test mallory H :0 Mallory Example",
    );
    assert!(!refreshed && responded);
    assert!(session.state().user("mallory").is_none());
}

#[test]
fn join_sync_and_errors_use_the_named_concurrent_channel() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    let papo = session
        .request_with_matcher(
            now,
            Line::join("#Papo["),
            ResponseMatcher::Join("#Papo[".into()),
        )
        .unwrap();
    let invite = session
        .request_with_matcher(
            now,
            Line::join("#invite"),
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
fn join_refusals_resolve_the_matching_channel() {
    let now = Instant::now();
    let cases = [
        (
            "#full",
            ":ergo.test 471 papo #full :Cannot join channel (+l)",
            Refusal::ChannelIsFull,
        ),
        (
            "#invite",
            ":ergo.test 473 papo #invite :Cannot join channel (+i)",
            Refusal::InviteOnlyChannel,
        ),
        (
            "#banned",
            ":irc.example.org 474 papo #banned :Cannot join channel (+b)",
            Refusal::BannedFromChannel,
        ),
        (
            "#keyed",
            ":ergo.test 475 papo #keyed :Cannot join channel (+k)",
            Refusal::BadChannelKey,
        ),
        (
            "#registered",
            ":irc.example.org 477 papo #registered :Cannot join channel (+r) - you need to be identified with services",
            Refusal::AccountRequired,
        ),
    ];

    for (channel, line, refusal) in cases {
        let mut session = Session::new(Config::default().nick("papo"));
        let id = session
            .request_with_matcher(
                now,
                Line::join(channel),
                ResponseMatcher::Join(channel.into()),
            )
            .unwrap();
        session.handle_line(now, line.as_bytes());

        let Some(Event::Response(response)) = session.poll_event() else {
            panic!("expected a response for {channel}")
        };
        assert_eq!(response.id, id);
        assert_eq!(response.error().unwrap().refusal(), refusal);
        assert_eq!(response.lines[0].as_ref().param(1), Some(channel));
        assert_eq!(session.poll_event(), None);
    }
}

#[test]
fn part_echoes_require_our_source_and_the_named_concurrent_channel() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    let papo = session
        .request_with_matcher(
            now,
            Line::new("PART", ["#Papo["]),
            ResponseMatcher::Part("#Papo[".into()),
        )
        .unwrap();
    let rust = session
        .request_with_matcher(
            now,
            Line::new("PART", ["#rust"]),
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
            Line::new("MODE", ["#Papo[", "+t"]),
            ResponseMatcher::Mode("#Papo[".into()),
        )
        .unwrap();
    let rust = session
        .request_with_matcher(
            now,
            Line::new("MODE", ["#rust", "+m"]),
            ResponseMatcher::Mode("#rust".into()),
        )
        .unwrap();
    for line in [
        ":ergo.test 482 papo #unknown :You're not a channel operator",
        ":ergo.test 482 papo #rust :You're not a channel operator",
        ":alice!alice@staff.example MODE #unknown +m",
        ":papo!papo@localhost MODE #papo{ +t",
        ":ergo.test PONG ergo.test :papo-mode-0",
        ":ergo.test PONG ergo.test :papo-mode-1",
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
            Line::new("TOPIC", ["#Papo["]),
            ResponseMatcher::Topic("#Papo[".into()),
        )
        .unwrap();
    let rust = session
        .request_with_matcher(
            now,
            Line::new("TOPIC", ["#rust"]),
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
            Line::new("LIST", Vec::<String>::new()),
            ResponseMatcher::List,
        )
        .unwrap();
    let second = session
        .request_with_matcher(now, Line::new("LIST", ["#rust"]), ResponseMatcher::List)
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
fn read_marker_replies_require_the_command_and_matching_target() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    let id = session
        .request_with_matcher(
            now,
            Line::new("MARKREAD", ["#Papo"]),
            ResponseMatcher::ReadMarker("#Papo".into()),
        )
        .unwrap();

    session.handle_line(now, b":ergo.test NOTICE #Papo :Scheduled maintenance");
    session.handle_line(
        now,
        b":ergo.test MARKREAD #rust timestamp=2026-10-20T12:00:00.000Z",
    );
    assert_no_response(&mut session);

    session.handle_line(
        now,
        b":ergo.test MARKREAD #papo timestamp=2026-10-20T12:00:01.000Z",
    );
    assert_eq!(response_id(&mut session), id);
}

#[test]
#[allow(clippy::too_many_lines)]
fn standard_failures_require_fail_kind_and_matching_command() {
    let now = Instant::now();
    let cases = [
        (
            "WHOIS",
            Line::new("WHOIS", ["alice"]),
            ResponseMatcher::Whois("alice".into()),
        ),
        (
            "WHO",
            Line::new("WHO", ["#papo"]),
            ResponseMatcher::Who("#papo".into()),
        ),
        (
            "JOIN",
            Line::join("#papo"),
            ResponseMatcher::Join("#papo".into()),
        ),
        (
            "PART",
            Line::new("PART", ["#papo"]),
            ResponseMatcher::Part("#papo".into()),
        ),
        (
            "MODE",
            Line::new("MODE", ["#papo", "+t"]),
            ResponseMatcher::Mode("#papo".into()),
        ),
        (
            "TOPIC",
            Line::new("TOPIC", ["#papo"]),
            ResponseMatcher::Topic("#papo".into()),
        ),
        (
            "KICK",
            Line::new("KICK", ["#papo", "spambot"]),
            ResponseMatcher::Kick {
                channel: "#papo".into(),
                nick: "spambot".into(),
            },
        ),
        (
            "INVITE",
            Line::new("INVITE", ["carol", "#papo"]),
            ResponseMatcher::Invite {
                channel: "#papo".into(),
                nick: "carol".into(),
            },
        ),
        (
            "NICK",
            Line::new("NICK", ["papo-away"]),
            ResponseMatcher::Nick("papo-away".into()),
        ),
        (
            "AWAY",
            Line::new("AWAY", ["reviewing patches"]),
            ResponseMatcher::Away(true),
        ),
        (
            "SETNAME",
            Line::new("SETNAME", ["Papo Test Client"]),
            ResponseMatcher::SetName("Papo Test Client".into()),
        ),
        (
            "OPER",
            Line::new("OPER", ["papo", "correct horse battery staple"]),
            ResponseMatcher::Oper,
        ),
        (
            "PING",
            Line::new("PING", ["papo-lag-42"]),
            ResponseMatcher::Ping("papo-lag-42".into()),
        ),
        (
            "LIST",
            Line::new("LIST", Vec::<String>::new()),
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
        if command == "MODE" {
            session.handle_line(now, b":ergo.test PONG ergo.test :papo-mode-0");
        }
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
            Line::new("WHOIS", ["alice"]),
            ResponseMatcher::Whois("alice".into()),
        )
        .unwrap();
    _ = session.poll_transmit();
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
                Line::new("WHOIS", [format!("user{index}")]),
                ResponseMatcher::Whois(format!("user{index}")),
            )
            .unwrap();
    }
    assert_eq!(
        session.request_with_matcher(
            now,
            Line::new("WHOIS", ["overflow"]),
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
    let echo = request(&mut session, now, Line::privmsg("#papo", "hello from papo"));
    let mode = request(&mut session, now, Line::new("MODE", ["#papo", "+i"]));
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
    let id = request(&mut session, now, Line::privmsg("#papo", "hello from papo"));
    session.handle_line(
        now,
        b"@label=0;msgid=RKi9Cw;time=2026-10-15T12:00:01.000Z :papo!~papo@localhost PRIVMSG #papo :hello from papo",
    );

    let Some(Event::Line(message)) = session.poll_event() else {
        panic!("expected confirming message before response")
    };
    assert_eq!(message.authorship(), papo_core::Authorship::Own);
    assert_eq!(message.as_ref().param(1), Some("hello from papo"));
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
    request(&mut session, now, Line::new("WHOIS", ["alice"]));
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
fn labelled_join_waits_for_state_sync_before_resolving() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = session
        .request_with_matcher(
            now,
            Line::join("#Papo["),
            ResponseMatcher::Join("#Papo[".into()),
        )
        .unwrap();
    session.handle_line(
        now,
        b"@label=0 :papo!~papo@localhost JOIN #papo{ papo :Papo Bot",
    );

    assert!(matches!(
        session.poll_event(),
        Some(Event::Joined { channel, nick, .. }) if channel == "#Papo[" && nick == "papo"
    ));
    assert_eq!(session.poll_event(), None);
    session.handle_line(now, b":ergo.test 353 papo = #papo{ :@papo alice");
    session.handle_line(now, b":ergo.test 366 papo #papo{ :End of /NAMES list");
    assert!(matches!(
        session.poll_event(),
        Some(Event::ChannelSynced { channel }) if channel == "#Papo["
    ));
    assert!(matches!(
        session.poll_event(),
        Some(Event::Response(response)) if response.id == id && response.lines.len() == 2
    ));
    assert!(session.state().channel("#Papo[").unwrap().is_synced());
}

#[test]
fn labelled_join_times_out_once_when_sync_never_finishes() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = session
        .request_with_matcher(
            now,
            Line::join("#Papo["),
            ResponseMatcher::Join("#Papo[".into()),
        )
        .unwrap();
    _ = session.poll_transmit();
    session.handle_line(
        now + std::time::Duration::from_secs(29),
        b"@label=0 :papo!~papo@localhost JOIN #papo{ * :Papo Bot",
    );
    while session.poll_event().is_some() {}

    session.handle_timeout(now + std::time::Duration::from_secs(30));
    assert!(std::iter::from_fn(|| session.poll_event()).all(
        |event| !matches!(event, Event::RequestTimedOut { id: timed_out } if timed_out == id)
    ));
    session.handle_timeout(now + std::time::Duration::from_secs(59));
    assert_eq!(
        std::iter::from_fn(|| session.poll_event())
            .filter(|event| matches!(event, Event::RequestTimedOut { id: timed_out } if *timed_out == id))
            .count(),
        1
    );
}

#[test]
fn labelled_join_without_an_echo_times_out_once() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = session
        .request_with_matcher(
            now,
            Line::join("#papo"),
            ResponseMatcher::Join("#papo".into()),
        )
        .unwrap();
    _ = session.poll_transmit();

    session.handle_timeout(now + std::time::Duration::from_secs(30));
    assert_eq!(
        std::iter::from_fn(|| session.poll_event())
            .filter(|event| matches!(event, Event::RequestTimedOut { id: timed_out } if *timed_out == id))
            .count(),
        1
    );
}

#[test]
fn own_part_or_kick_ends_a_labelled_join_during_sync() {
    for departure in [
        ":papo!~papo@localhost PART #papo :Registration required",
        ":ergo.test KICK #papo papo :Channel is full",
    ] {
        let now = Instant::now();
        let mut session = labelled_session(now);
        let id = session
            .request_with_matcher(
                now,
                Line::join("#papo"),
                ResponseMatcher::Join("#papo".into()),
            )
            .unwrap();
        session.handle_line(
            now,
            b"@label=0 :papo!~papo@localhost JOIN #papo * :Papo Bot",
        );
        session.handle_line(now, departure.as_bytes());

        let response = std::iter::from_fn(|| session.poll_event()).find_map(|event| match event {
            Event::Response(response) if response.id == id => Some(response),
            _ => None,
        });
        assert_eq!(response.unwrap().lines.len(), 2);
    }
}

#[test]
fn other_join_departures_do_not_end_a_labelled_join() {
    for departure in [
        ":alice!alice@staff.example PART #papo :Leaving",
        ":papo!~papo@localhost PART #rust :Registration required",
        ":ergo.test KICK #papo alice :Channel is full",
        ":ergo.test KICK #rust papo :Channel is full",
    ] {
        let now = Instant::now();
        let mut session = labelled_session(now);
        let id = session
            .request_with_matcher(
                now,
                Line::join("#papo"),
                ResponseMatcher::Join("#papo".into()),
            )
            .unwrap();
        session.handle_line(
            now,
            b"@label=0 :papo!~papo@localhost JOIN #papo * :Papo Bot",
        );
        session.handle_line(now, departure.as_bytes());

        assert!(
            std::iter::from_fn(|| session.poll_event())
                .all(|event| !matches!(event, Event::Response(response) if response.id == id)),
            "irrelevant departure resolved the join: {departure}"
        );
        session.handle_line(now, b":ergo.test KICK #papo papo :Channel is full");
        assert!(
            std::iter::from_fn(|| session.poll_event())
                .any(|event| matches!(event, Event::Response(response) if response.id == id)),
            "matching departure did not resolve the join after: {departure}"
        );
    }
}

#[test]
fn resolves_a_recorded_ergo_whois_batch_once() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = request(&mut session, now, Line::new("WHOIS", ["alice"]));
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
fn request_deadline_starts_when_its_rate_limited_line_is_transmitted() {
    let now = Instant::now();
    let rate = RateLimit::new(1, std::time::Duration::from_secs(60))
        .unwrap()
        .strict();
    let config = Config::default().rate_limit(rate);
    let mut session = Session::new(config);
    session.handle_connected(now);
    session.handle_line(now, b":ergo.test CAP * LS :labeled-response");
    session.handle_line(now, b":ergo.test CAP * ACK :labeled-response");
    while session.poll_transmit().is_some() {}
    let alice = request(&mut session, now, Line::new("WHOIS", ["alice"]));
    let bob = request(&mut session, now, Line::new("WHOIS", ["bob"]));
    assert!(session.poll_transmit().is_some());
    assert_eq!(session.poll_transmit(), None);
    session.handle_timeout(now + std::time::Duration::from_secs(30));
    assert!(matches!(session.poll_event(), Some(Event::RequestTimedOut { id }) if id == alice));
    assert_eq!(session.poll_event(), None);
    session.handle_timeout(now + std::time::Duration::from_secs(60));
    assert_eq!(session.poll_transmit().unwrap(), b"@label=1 WHOIS bob\r\n");
    session.handle_timeout(now + std::time::Duration::from_secs(90));
    assert!(
        std::iter::from_fn(|| session.poll_event())
            .any(|event| matches!(event, Event::RequestTimedOut { id } if id == bob))
    );
}

#[test]
fn unlabelled_request_deadlines_start_one_transmission_at_a_time() {
    let now = Instant::now();
    let rate = RateLimit::new(1, std::time::Duration::from_secs(60))
        .unwrap()
        .strict();
    let mut session = Session::new(Config::default().rate_limit(rate));
    let alice = session
        .request_with_matcher(
            now,
            Line::new("WHOIS", ["alice"]),
            ResponseMatcher::Whois("alice".into()),
        )
        .unwrap();
    let bob = session
        .request_with_matcher(
            now,
            Line::new("WHOIS", ["bob"]),
            ResponseMatcher::Whois("bob".into()),
        )
        .unwrap();

    assert_eq!(session.poll_transmit().unwrap(), b"WHOIS alice\r\n");
    assert_eq!(session.poll_transmit(), None);
    session.handle_timeout(now + std::time::Duration::from_secs(30));
    assert!(matches!(session.poll_event(), Some(Event::RequestTimedOut { id }) if id == alice));
    assert_eq!(session.poll_event(), None);

    session.handle_timeout(now + std::time::Duration::from_secs(60));
    assert_eq!(session.poll_transmit().unwrap(), b"WHOIS bob\r\n");
    session.handle_timeout(now + std::time::Duration::from_secs(90));
    assert!(matches!(session.poll_event(), Some(Event::RequestTimedOut { id }) if id == bob));
}

#[test]
fn pending_non_history_batch_dispatches_protocol_state() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = request(&mut session, now, Line::join("#papo"));
    session.handle_line(now, b"@label=0 :ergo.test BATCH +join-42 labeled-response");
    session.handle_line(
        now,
        b"@batch=join-42 :papo!~papo@localhost JOIN #papo papo :Papo Bot",
    );
    session.handle_line(now, b":ergo.test BATCH -join-42");

    assert!(matches!(
        session.poll_event(),
        Some(Event::Joined { channel, nick, .. }) if channel == "#papo" && nick == "papo"
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
        Line::new("CHATHISTORY", ["LATEST", "#papo", "*", "50"]),
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
        Some(Event::History { conversation: Conversation::Channel(channel), items, end, .. })
            if channel == "#papo" && items.len() == 1 && end
    ));
    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("expected correlated history response")
    };
    assert_eq!(response.id, id);
    assert_eq!(response.lines.len(), 1);
    let history = response.history.expect("history reference");
    assert!(matches!(history.conversation, Conversation::Channel(channel) if channel == "#papo"));
    assert_eq!(history.items.len(), 1);
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
        Some(Event::History { conversation: Conversation::Channel(channel), items, .. }) if channel == "#papo" && items.len() == 1
    ));
    assert!(matches!(
        session.poll_event(),
        Some(Event::Line(message))
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
            Some(Event::Line(message)) if message.batch_type() == Some(batch_type)
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
        Line::new("CHATHISTORY", ["LATEST", "#papo", "bad", "50"]),
    );
    request(&mut session, now, Line::new("WHOIS", ["missing"]));
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
    assert_eq!(fail.error().unwrap().refusal(), Refusal::InvalidParameters);
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
    assert_eq!(numeric.error().unwrap().refusal(), Refusal::NoSuchNick);
}

#[expect(
    clippy::too_many_lines,
    reason = "one readable transcript case is kept for every numeric"
)]
#[test]
fn classifies_numeric_refusals() {
    let now = Instant::now();
    let cases = [
        (
            "400 papo NICK :Nickname is already in use",
            Refusal::CommandFailed,
        ),
        ("401 papo alice :No such nick", Refusal::NoSuchNick),
        (
            "402 papo irc.example.net :No such server",
            Refusal::NoSuchServer,
        ),
        ("403 papo #missing :No such channel", Refusal::NoSuchChannel),
        (
            "406 papo alice :There was no such nickname",
            Refusal::NoNicknameHistory,
        ),
        ("409 papo :No origin specified", Refusal::MissingOrigin),
        (
            "411 papo :No recipient given (PRIVMSG)",
            Refusal::MissingRecipient,
        ),
        ("422 papo :MOTD File is missing", Refusal::MotdUnavailable),
        ("431 papo :No nickname given", Refusal::NicknameRequired),
        (
            "432 papo 42bad :Erroneous nickname",
            Refusal::ErroneousNickname,
        ),
        (
            "433 papo alice :Nickname is already in use",
            Refusal::NicknameInUse,
        ),
        (
            "441 papo alice #papo :They aren't on that channel",
            Refusal::UserNotInChannel,
        ),
        (
            "442 papo #papo :You're not on that channel",
            Refusal::NotOnChannel,
        ),
        (
            "443 papo alice #papo :is already on channel",
            Refusal::UserOnChannel,
        ),
        (
            "461 papo WHOIS :Not enough parameters",
            Refusal::MissingParameters,
        ),
        (
            "462 papo :You may not reregister",
            Refusal::AlreadyConnected,
        ),
        ("464 papo :Password incorrect", Refusal::PasswordIncorrect),
        (
            "465 papo :You are banned from this server",
            Refusal::BannedFromServer,
        ),
        (
            "471 papo #papo :Cannot join channel (+l)",
            Refusal::ChannelIsFull,
        ),
        (
            "472 papo z :is unknown mode char to me",
            Refusal::UnknownMode,
        ),
        (
            "473 papo #papo :Cannot join channel (+i)",
            Refusal::InviteOnlyChannel,
        ),
        (
            "474 papo #papo :Cannot join channel (+b)",
            Refusal::BannedFromChannel,
        ),
        (
            "475 papo #papo :Cannot join channel (+k)",
            Refusal::BadChannelKey,
        ),
        (
            "478 papo #papo b :Channel list is full",
            Refusal::BanListFull,
        ),
        (
            "481 papo :Permission Denied- You're not an IRC operator",
            Refusal::NoPrivileges,
        ),
        (
            "482 papo #papo :You're not channel operator",
            Refusal::ChannelOperatorPrivilegesNeeded,
        ),
        (
            "483 papo :You cant kill a server!",
            Refusal::CannotKillServer,
        ),
        (
            "484 papo alice #papo :Cannot kick protected user",
            Refusal::CannotKickProtected,
        ),
        (
            "524 papo MASKMATCH :No help available on this topic",
            Refusal::NoHelpAvailable,
        ),
        (
            "525 papo #papo :Key is not well-formed",
            Refusal::InvalidKey,
        ),
        ("691 papo :TLS is not configured", Refusal::TlsUpgradeFailed),
        (
            "697 papo #papo alice!*@* b :Channel #papo list already contains alice!*@*",
            Refusal::ListModeAlreadySet,
        ),
        (
            "698 papo #papo alice!*@* b :Channel #papo list does not contain alice!*@*",
            Refusal::ListModeNotSet,
        ),
        (
            "723 papo kill:remote :Insufficient oper privileges.",
            Refusal::InsufficientPrivileges,
        ),
        (
            "902 papo :You must use a nick assigned to you",
            Refusal::AccountLocked,
        ),
        ("904 papo :SASL authentication failed", Refusal::SaslFailed),
        (
            "905 papo :SASL message too long",
            Refusal::SaslMessageTooLong,
        ),
        (
            "906 papo :SASL authentication aborted",
            Refusal::SaslAborted,
        ),
        (
            "907 papo :You have already authenticated using SASL",
            Refusal::AlreadyAuthenticated,
        ),
        (
            "404 papo #rust :Cannot send to channel",
            Refusal::CannotSendToChannel,
        ),
        (
            "405 papo #staff :You have joined too many channels",
            Refusal::TooManyChannels,
        ),
        ("412 papo :No text to send", Refusal::NoTextToSend),
        (
            "417 papo PRIVMSG :Input line was too long",
            Refusal::InputTooLong,
        ),
        (
            "421 papo FROBULATE :Unknown command",
            Refusal::UnknownCommand,
        ),
        (
            "436 papo alice :Nickname collision KILL",
            Refusal::NickCollision,
        ),
        (
            "437 papo #staff :Nick/channel is temporarily unavailable",
            Refusal::TemporarilyUnavailable,
        ),
        (
            "451 papo :You have not registered",
            Refusal::NotConnectedYet,
        ),
        (
            "476 papo #bad:mask :Bad Channel Mask",
            Refusal::BadChannelMask,
        ),
        (
            "477 papo #registered :Cannot join channel (+r) - you need to be identified with services",
            Refusal::AccountRequired,
        ),
        (
            "489 papo #secure :Cannot join channel (+z)",
            Refusal::SecureOnlyChannel,
        ),
        ("491 papo :No O-lines for your host", Refusal::NoOperHost),
        ("501 papo :Unknown MODE flag", Refusal::UnknownUserMode),
        (
            "502 papo :Cannot change mode for other users",
            Refusal::UsersDontMatch,
        ),
        (
            "696 papo #rust k * :Invalid mode parameter",
            Refusal::InvalidModeParameter,
        ),
        (
            "263 papo WHO :Server load is temporarily too heavy",
            Refusal::TryAgain,
        ),
    ];

    for (line, expected) in cases {
        let mut session = labelled_session(now);
        session.handle_line(
            now,
            b":ergo.test 001 papo :Welcome to the ErgoTest IRC Network",
        );
        session.handle_line(now, b":ergo.test 422 papo :MOTD File is missing");
        while session.poll_event().is_some() {}
        request(&mut session, now, Line::new("RAW", ["probe"]));
        let server = match line.get(..3) {
            Some("484") => "irc.unreal.test",
            Some("409" | "411" | "483" | "691" | "723") => "irc.solanum.test",
            _ => "ergo.test",
        };
        session.handle_line(now, format!("@label=0 :{server} {line}").as_bytes());
        let response = loop {
            match session.poll_event() {
                Some(Event::Response(response)) => break response,
                Some(_) => {}
                None => panic!("expected refusal response for {line}"),
            }
        };
        assert_eq!(response.error().unwrap().refusal(), expected, "{line}");
    }
}

#[test]
fn times_out_and_releases_a_label_for_late_delivery() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = request(&mut session, now, Line::new("WHOIS", ["alice"]));
    _ = session.poll_transmit();
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
        matches!(session.poll_event(), Some(Event::Line(message)) if message.as_ref().command_is("401"))
    );
}

#[test]
fn aborts_pending_requests_on_disconnect() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = request(&mut session, now, Line::new("WHOIS", ["alice"]));
    session.handle_disconnected(now);
    assert!(
        matches!(session.poll_event(), Some(Event::RequestAborted { id: aborted, transmitted: false }) if aborted == id)
    );
}

#[test]
fn labelled_join_aborts_once_before_any_bytes_are_transmitted() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let id = session
        .request_with_matcher(
            now,
            Line::join("#papo"),
            ResponseMatcher::Join("#papo".into()),
        )
        .unwrap();

    session.handle_disconnected(now);
    let aborted = std::iter::from_fn(|| session.poll_event())
        .filter_map(|event| match event {
            Event::RequestAborted { id, transmitted } => Some((id, transmitted)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(aborted, [(id, false)]);
}

#[test]
fn abort_reports_when_a_labelled_privmsg_reached_the_transport() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    session.handle_connected(now);
    session.handle_line(now, b":ergo.test CAP * LS :echo-message labeled-response");
    session.handle_line(now, b":ergo.test CAP * ACK :echo-message labeled-response");
    while session.poll_transmit().is_some() {}
    let id = session
        .send_text_detailed(now, TextKind::Privmsg, "#papo", "deploy complete", &[])
        .unwrap()
        .request
        .unwrap();

    assert_eq!(
        session.poll_transmit().unwrap(),
        b"@label=0 PRIVMSG #papo :deploy complete\r\n"
    );
    session.handle_disconnected(now);
    assert!(matches!(
        session.poll_event(),
        Some(Event::RequestAborted {
            id: aborted,
            transmitted: true,
        }) if aborted == id
    ));
}

#[test]
fn partially_transmitted_split_send_aborts_as_transmitted() {
    let now = Instant::now();
    let mut session = echo_session(now);
    let text = "Production deployment remains healthy across all regions. ".repeat(12);
    let sent = session
        .send_text_detailed(now, TextKind::Privmsg, "#papo", &text, &[])
        .unwrap();
    assert_eq!(sent.confirmations, 2);
    let id = sent.request.unwrap();

    assert!(session.poll_transmit().is_some());
    session.handle_disconnected(now);
    let aborted = std::iter::from_fn(|| session.poll_event())
        .filter_map(|event| match event {
            Event::RequestAborted { id, transmitted } => Some((id, transmitted)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(aborted, [(id, true)]);
}

#[test]
fn split_send_timeout_and_disconnect_are_each_reported_once() {
    let now = Instant::now();
    let text = "Production deployment remains healthy across all regions. ".repeat(12);

    let mut timed_out = echo_session(now);
    let send = timed_out
        .send_text_detailed(now, TextKind::Privmsg, "#papo", &text, &[])
        .unwrap();
    assert_eq!(send.confirmations, 2);
    let id = send.request.unwrap();
    while timed_out.poll_transmit().is_some() {}
    timed_out.handle_timeout(now + std::time::Duration::from_secs(30));
    let timeouts = std::iter::from_fn(|| timed_out.poll_event())
        .filter_map(|event| match event {
            Event::RequestTimedOut { id } => Some(id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(timeouts, [id]);

    let mut disconnected = echo_session(now);
    let send = disconnected
        .send_text_detailed(now, TextKind::Privmsg, "#papo", &text, &[])
        .unwrap();
    assert_eq!(send.confirmations, 2);
    let id = send.request.unwrap();
    while disconnected.poll_transmit().is_some() {}
    disconnected.handle_disconnected(now);
    let aborted = std::iter::from_fn(|| disconnected.poll_event())
        .filter_map(|event| match event {
            Event::RequestAborted { id, transmitted } => Some((id, transmitted)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(aborted, [(id, true)]);
}

#[test]
fn one_timeout_expires_every_ready_request() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let ids: Vec<_> = ["alice", "bob", "carol"]
        .into_iter()
        .map(|nick| request(&mut session, now, Line::new("WHOIS", [nick])))
        .collect();
    while session.poll_transmit().is_some() {}

    session.handle_timeout(now + std::time::Duration::from_secs(30));

    let timed_out: Vec<_> = std::iter::from_fn(|| session.poll_event())
        .filter_map(|event| match event {
            Event::RequestTimedOut { id } => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(timed_out, ids);
    session.handle_timeout(now + std::time::Duration::from_secs(30));
    assert!(
        std::iter::from_fn(|| session.poll_event())
            .all(|event| !matches!(event, Event::RequestTimedOut { .. }))
    );
}

#[test]
fn bounds_pending_requests_at_the_literal_limit() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    for nick in 0..256 {
        session
            .request(now, Line::new("WHOIS", [format!("user{nick}")]))
            .unwrap();
    }
    assert_eq!(
        session.request(now, Line::new("WHOIS", ["overflow"])),
        Err(RequestError::PendingLimit)
    );
}

#[test]
fn labelled_join_fallbacks_obey_the_literal_pending_limit() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    for channel in 0..256 {
        let channel = format!("#project-{channel}");
        session
            .request_with_matcher(now, Line::join(&channel), ResponseMatcher::Join(channel))
            .unwrap();
        let line = String::from_utf8(session.poll_transmit().unwrap()).unwrap();
        let label = LineRef::parse(&line)
            .unwrap()
            .tag("label")
            .unwrap()
            .into_owned();
        session.handle_line(now, format!("@label={label} :ergo.test ACK").as_bytes());
    }

    assert_eq!(
        session.request_with_matcher(
            now,
            Line::join("#overflow"),
            ResponseMatcher::Join("#overflow".into()),
        ),
        Err(RequestError::PendingLimit)
    );
    assert_eq!(session.poll_transmit(), None);
    session.handle_disconnected(now);
    assert_eq!(
        std::iter::from_fn(|| session.poll_event())
            .filter(|event| matches!(event, Event::RequestAborted { .. }))
            .count(),
        256
    );
}

#[test]
fn generates_one_thousand_distinct_short_alphanumeric_labels() {
    let now = Instant::now();
    let mut session = labelled_session(now);
    let mut labels = std::collections::BTreeSet::new();
    for nick in 0..1_000 {
        session
            .request(now, Line::new("WHOIS", [format!("user{nick}")]))
            .unwrap();
        let line = String::from_utf8(session.poll_transmit().unwrap()).unwrap();
        let message = LineRef::parse(&line).unwrap();
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
        matches!(session.poll_event(), Some(Event::Line(message)) if message.as_ref().command_is("WARN"))
    );
}
