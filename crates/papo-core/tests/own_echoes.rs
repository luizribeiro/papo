//! Transcript coverage for requests that a server confirms by echoing our own command.

use std::time::Instant;

use papo_core::{Config, Event, RateLimit, Response, ResponseMatcher, Session};
use papo_proto::{Line, LineRef};

const ERGO_ISUPPORT: &str = ":ergo.test 005 papo CASEMAPPING=ascii CHANTYPES=# PREFIX=(qaohv)~&@%+ :are supported by this server";

fn ergo(now: Instant, caps: &str) -> Session {
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited()),
    );
    session.handle_connected(now);
    if !caps.is_empty() {
        session.handle_line(now, format!(":ergo.test CAP * LS :{caps}").as_bytes());
        session.handle_line(now, format!(":ergo.test CAP * ACK :{caps}").as_bytes());
    }
    for line in [
        ":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo",
        ERGO_ISUPPORT,
        ":ergo.test 422 papo :MOTD File is missing",
        "@msgid=drk9tpzyy22xa592x2q3f7j3ri;time=2026-11-15T20:07:04.645Z :papo!~u@3t4fgnf9zitka.irc JOIN #papo",
        ":ergo.test 353 papo = #papo :@papo bob",
        ":ergo.test 366 papo #papo :End of NAMES list",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
    session
}

fn response(session: &mut Session) -> Option<Response> {
    std::iter::from_fn(|| session.poll_event()).find_map(|event| match event {
        Event::Response(response) => Some(response),
        _ => None,
    })
}

struct Case {
    command: Line,
    matcher: ResponseMatcher,
    labelled: &'static str,
    unlabelled: &'static str,
}

fn cases() -> [Case; 5] {
    [
        Case {
            command: Line::new("NICK").with_params(["Papo2"]),
            matcher: ResponseMatcher::Nick("Papo2".into()),
            labelled: "@time=2026-11-15T20:07:08.649Z;label=0;msgid=7kbftprgq82kf8a6ckkikgvr9w :papo!~u@3t4fgnf9zitka.irc NICK Papo2",
            unlabelled: "@msgid=ah547zsvi38aatxwpcfpsj8kxn;time=2026-11-15T20:07:11.652Z :papo!~u@3t4fgnf9zitka.irc NICK Papo2",
        },
        Case {
            command: Line::new("SETNAME").with_params(["Carol  bold "]),
            matcher: ResponseMatcher::SetName,
            labelled: "@time=2026-11-15T20:07:13.153Z;label=0 :papo!~u@3t4fgnf9zitka.irc SETNAME :Carol  bold ",
            unlabelled: "@time=2026-11-15T20:08:34.189Z :papo!~u@3t4fgnf9zitka.irc SETNAME :Carol  bold ",
        },
        Case {
            command: Line::new("TOPIC").with_params(["#papo", "hello topic"]),
            matcher: ResponseMatcher::Topic("#papo".into()),
            labelled: "@msgid=bc6raycgzeaqzat395ckz8b5bi;time=2026-11-15T20:07:16.155Z;label=0 :papo!~u@3t4fgnf9zitka.irc TOPIC #papo :hello topic",
            unlabelled: "@msgid=48mgj9m3n6593xxqv2jtbyga3e;time=2026-11-15T20:07:20.659Z :papo!~u@3t4fgnf9zitka.irc TOPIC #papo :hello topic",
        },
        Case {
            command: Line::new("KICK").with_params(["#papo", "bob", "bye"]),
            matcher: ResponseMatcher::__kick("#papo".into(), "bob".into()),
            labelled: "@time=2026-11-15T20:07:22.160Z;label=0;msgid=k8gq9fd2udzi77ghui3v88y482 :papo!~u@3t4fgnf9zitka.irc KICK #papo bob bye",
            unlabelled: "@msgid=k8gq9fd2udzi77ghui3v88y482;time=2026-11-15T20:07:22.160Z :papo!~u@3t4fgnf9zitka.irc KICK #papo bob bye",
        },
        Case {
            command: Line::new("PART").with_params(["#papo", "leaving"]),
            matcher: ResponseMatcher::Part("#papo".into()),
            labelled: "@msgid=x95ke2uf27w64gbhhcnccekj8s;time=2026-11-15T20:07:35.242Z;label=0 :papo!~u@3t4fgnf9zitka.irc PART #papo leaving",
            unlabelled: "@msgid=x95ke2uf27w64gbhhcnccekj8s;time=2026-11-15T20:07:35.242Z :papo!~u@3t4fgnf9zitka.irc PART #papo leaving",
        },
    ]
}

fn assert_echo_kept(response: Option<Response>, echo: &str) {
    let response = response.unwrap_or_else(|| panic!("{echo} should end its request"));
    let expected = LineRef::parse(echo).unwrap();
    let [kept] = response.lines.as_slice() else {
        panic!(
            "{echo} should be the only line kept, got {:?}",
            response.lines
        );
    };
    let kept = kept.as_ref();
    assert_eq!(kept.command(), expected.command(), "{echo}");
    assert!(kept.params().eq(expected.params()), "{echo}");
    for tag in ["msgid", "time"] {
        assert_eq!(kept.tag(tag), expected.tag(tag), "{tag} of {echo}");
    }
}

#[test]
fn a_labelled_response_keeps_the_ergo_echo_of_our_command() {
    let now = Instant::now();
    for case in cases() {
        let mut session = ergo(now, "labeled-response message-tags server-time setname");
        session.request(now, case.command).unwrap();
        session.handle_line(now, case.labelled.as_bytes());
        assert_echo_kept(response(&mut session), case.labelled);
    }
}

#[test]
fn an_unlabelled_response_keeps_the_ergo_echo_that_matched_it() {
    let now = Instant::now();
    for case in cases() {
        let mut session = ergo(now, "message-tags server-time setname");
        session
            .request_with_matcher(now, case.command, case.matcher)
            .unwrap();
        session.handle_line(now, case.unlabelled.as_bytes());
        assert_echo_kept(response(&mut session), case.unlabelled);
    }
}

const CHANOP_KICK: &str = "@msgid=5ia6vq2kbpzgbsmpq9kbdrm7kc;time=2026-11-15T20:07:21.402Z :chanop!~u@9f3xcwqwgz5yq.irc KICK #papo bob :spam";

struct KickMode {
    caps: &'static str,
    label: &'static str,
}

const KICK_MODES: [KickMode; 2] = [
    KickMode {
        caps: "labeled-response message-tags server-time",
        label: "label=0;",
    },
    KickMode {
        caps: "message-tags server-time",
        label: "",
    },
];

fn kick_bob(now: Instant, mode: &KickMode) -> Session {
    let mut session = ergo(now, mode.caps);
    session
        .request_with_matcher(
            now,
            Line::new("KICK").with_params(["#papo", "bob", "bye"]),
            ResponseMatcher::__kick("#papo".into(), "bob".into()),
        )
        .unwrap();
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
    session
}

fn handle_events(session: &mut Session, now: Instant, line: &str) -> Vec<Event> {
    session.handle_line(now, line.as_bytes());
    std::iter::from_fn(|| session.poll_event()).collect()
}

fn responses(events: &[Event]) -> Vec<&Response> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Response(response) => Some(response),
            _ => None,
        })
        .collect()
}

#[test]
fn another_operators_kick_leaves_our_kick_pending_until_our_echo() {
    let now = Instant::now();
    for mode in &KICK_MODES {
        let mut session = kick_bob(now, mode);
        let foreign = handle_events(&mut session, now, CHANOP_KICK);
        assert!(responses(&foreign).is_empty(), "{}", mode.caps);
        assert!(
            foreign
                .iter()
                .any(|event| matches!(event, Event::Kicked { .. })),
            "{}",
            mode.caps
        );

        let echo = format!(
            "@{}msgid=k8gq9fd2udzi77ghui3v88y482;time=2026-11-15T20:07:22.160Z :papo!~u@3t4fgnf9zitka.irc KICK #papo bob bye",
            mode.label
        );
        assert_echo_kept(
            responses(&handle_events(&mut session, now, &echo))
                .pop()
                .cloned(),
            &echo,
        );
    }
}

#[test]
fn a_refused_kick_is_not_completed_by_another_operators_kick() {
    let now = Instant::now();
    for mode in &KICK_MODES {
        let mut session = kick_bob(now, mode);
        assert!(responses(&handle_events(&mut session, now, CHANOP_KICK)).is_empty());

        let refusal = format!(
            "@{}time=2026-11-15T20:07:22.160Z :ergo.test 482 papo #papo :You're not a channel operator",
            mode.label
        );
        let events = handle_events(&mut session, now, &refusal);
        let [response] = responses(&events)[..] else {
            panic!("482 should end the kick under {}", mode.caps);
        };
        let [line] = response.lines.as_slice() else {
            panic!(
                "only the refusal belongs to the kick, got {:?}",
                response.lines
            );
        };
        let expected = LineRef::parse(&refusal).unwrap();
        assert_eq!(line.as_ref().command(), expected.command(), "{}", mode.caps);
    }
}

#[test]
fn our_kick_echo_matches_our_nick_under_the_server_casemapping() {
    let now = Instant::now();
    let mut session = kick_bob(now, &KICK_MODES[1]);
    let echo = "@time=2026-11-15T20:07:22.160Z :PaPo!~u@3t4fgnf9zitka.irc KICK #Papo BOB :bye";
    let events = handle_events(&mut session, now, echo);
    assert_eq!(responses(&events).len(), 1);
}
