//! Transcript coverage for the person a `WHO` or `WHOIS` row describes.
//!
//! The row's subject leaves or renames, and another user takes the nickname, before the reply
//! completes, so a token read when the reply is mapped would name the newcomer.

use std::time::Instant;

use papo_core::{Config, Event, RateLimit, Response, ResponseMatcher, Session};
use papo_proto::Line;

struct Reply {
    command: &'static str,
    params: &'static [&'static str],
    row: &'static str,
    end: &'static str,
}

const REPLIES: [Reply; 3] = [
    Reply {
        command: "WHO",
        params: &["alice", "%tcuhnfar,152"],
        row: ":ergo.test 354 papo 152 #review ~alice cloak.example alice H alice-account :Alice Liddell",
        end: ":ergo.test 315 papo alice :End of WHO list",
    },
    Reply {
        command: "WHO",
        params: &["alice"],
        row: ":ergo.test 352 papo #review ~alice cloak.example ergo.test alice H :0 Alice Liddell",
        end: ":ergo.test 315 papo alice :End of WHO list",
    },
    Reply {
        command: "WHOIS",
        params: &["alice"],
        row: ":ergo.test 311 papo alice ~alice cloak.example * :Alice Liddell",
        end: ":ergo.test 318 papo alice :End of /WHOIS list",
    },
];

const QUIT: &str = ":alice!~alice@cloak.example QUIT :Quit: bye";
const RENAME: &str = ":alice!~alice@cloak.example NICK alice_away";
const TAKEOVER: &str = ":mallory!~mallory@users.example NICK alice";

fn session(labelled: bool) -> Session {
    let now = Instant::now();
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    if labelled {
        session.handle_connected(now);
        session.handle_line(now, b"CAP * LS :batch labeled-response");
        session.handle_line(now, b"CAP * ACK :batch labeled-response");
    }
    for line in [
        ":ergo.test 001 papo :Welcome to the Ergo test network",
        ":ergo.test 422 papo :MOTD File is missing",
        ":papo!~papo@localhost JOIN #review",
        ":alice!~alice@cloak.example JOIN #review",
        ":mallory!~mallory@users.example JOIN #review",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    while session.poll_event().is_some() {}
    while session.poll_transmit().is_some() {}
    session
}

fn response(session: &mut Session, reply: &Reply, labelled: bool, between: &[&str]) -> Response {
    let now = Instant::now();
    let matcher = if reply.command == "WHOIS" {
        ResponseMatcher::Whois("alice".into())
    } else {
        ResponseMatcher::Who("alice".into())
    };
    let line = Line::new(reply.command).with_params(reply.params.iter().copied());
    session.request_with_matcher(now, line, matcher).unwrap();
    let sent = session.poll_transmit().unwrap();
    let sent = String::from_utf8_lossy(sent.as_ref()).into_owned();
    let lines = [&[reply.row][..], between, &[reply.end]].concat();
    let lines = if labelled {
        let label = sent
            .strip_prefix("@label=")
            .and_then(|rest| rest.split_once(' '))
            .map(|(label, _)| label)
            .unwrap();
        std::iter::once(format!(
            "@label={label} :ergo.test BATCH +r labeled-response"
        ))
        .chain(lines.iter().map(|line| format!("@batch=r {line}")))
        .chain([":ergo.test BATCH -r".to_owned()])
        .collect()
    } else {
        lines
            .iter()
            .map(|line| (*line).to_owned())
            .collect::<Vec<_>>()
    };
    for line in lines {
        session.handle_line(now, line.as_bytes());
    }
    std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::Response(response) => Some(response),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{}: no response", reply.row))
}

#[test]
fn a_reply_row_records_the_person_it_describes_when_it_arrives() {
    for labelled in [false, true] {
        for reply in &REPLIES {
            for departure in [QUIT, RENAME] {
                let case = format!("{} labelled={labelled} after {departure}", reply.row);
                let mut session = session(labelled);
                let alice = session.state().__reply_token_for("alice");
                let response = response(&mut session, reply, labelled, &[departure, TAKEOVER]);
                let newcomer = session.state().__reply_token_for("alice");
                assert_ne!(alice, newcomer, "{case}: no handoff happened");

                assert_eq!(response.__subject_token(0), Some(alice), "{case}");
                let end = response.lines.len() - 1;
                assert_eq!(response.__subject_token(end), None, "{case}");
            }
        }
    }
}
