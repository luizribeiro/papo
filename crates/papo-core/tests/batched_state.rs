//! Transcript coverage for live state changes and replay isolation across batch endings.

use std::time::{Duration, Instant};

use papo_core::{AccountStatus, Config, RateLimit, Session, State};
use papo_proto::Line;

const MEMBERS: &[&str] = &[
    ":ChanServ!ChanServ@services.int QUIT :irc.papo.test services.papo.test",
    ":ChanServ!ChanServ@services.int NICK ChanServ-away",
    ":ChanServ!ChanServ@services.int PART #papo :Leaving",
    ":operator!~op@staff.example KICK #papo ChanServ :Channel maintenance",
    ":alice!~alice@staff.example JOIN #papo alice :Alice",
    ":ChanServ!ChanServ@services.int ACCOUNT *",
    ":ChanServ!ChanServ@services.int CHGHOST ChanServ services.example",
    ":operator!~op@staff.example MODE #papo +o ChanServ",
];

fn applied(state: &State, line: &str) -> bool {
    let channel = state.channel("#papo").unwrap();
    let chanserv = || state.user("ChanServ").unwrap();
    match line.split_ascii_whitespace().nth(1).unwrap() {
        "QUIT" => state.user("ChanServ").is_none(),
        "NICK" => state.user("ChanServ-away").is_some(),
        "PART" | "KICK" => channel.member("ChanServ").is_none(),
        "JOIN" => channel.member("alice").is_some(),
        "ACCOUNT" => chanserv().account() == &AccountStatus::LoggedOut,
        "CHGHOST" => chanserv().host() == Some("services.example"),
        "MODE" => !channel.member("ChanServ").unwrap().prefixes().is_empty(),
        _ => unreachable!(),
    }
}

#[derive(Clone, Copy, Debug)]
enum Ending {
    Close,
    Size,
    Timeout,
    OpenLimit,
}

fn open_response(session: &mut Session, now: Instant) {
    for line in [
        ":ergo.test CAP * LS :batch labeled-response message-tags",
        ":ergo.test CAP * ACK :batch labeled-response message-tags",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    while session.poll_transmit().is_some() {}
    session
        .request(now, Line::new("WHOIS").param("ChanServ"))
        .unwrap();
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"@label=0 WHOIS ChanServ\r\n"
    );
}

fn transcript(kind: &str, parent: Option<&str>, ending: Ending, member: &str, live: bool) {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .rate_limit(RateLimit::unlimited())
            .max_open_batches(2)
            .max_batch_lines(if matches!(ending, Ending::Size) { 1 } else { 8 })
            .batch_timeout(Duration::from_secs(1)),
    );
    let (tags, parameters) = match kind {
        "labeled-response" => {
            open_response(&mut session, now);
            ("@label=0 ", "")
        }
        "netsplit" | "netjoin" => ("", " irc.papo.test services.papo.test"),
        _ => ("", " #papo"),
    };
    let mut read = |line: &str| session.handle_line(now, line.as_bytes());
    read(":irc.papo.test 005 papo PREFIX=(ov)@+ :are supported by this server");
    read(":papo!~papo@localhost JOIN #papo");
    read(":ChanServ!ChanServ@services.int JOIN #papo chanserv :ChanServ");
    if matches!(ending, Ending::OpenLimit) {
        read(":irc.papo.test BATCH +hold vendor/sync");
        if parent.is_none() {
            read(":irc.papo.test BATCH +hold2 vendor/sync");
        }
    }
    let parent_tag = if let Some(parent) = parent {
        read(&format!(":irc.papo.test BATCH +outer {parent} #papo"));
        "@batch=outer "
    } else {
        tags
    };
    read(&format!(
        "{parent_tag}:irc.papo.test BATCH +inner {kind}{parameters}"
    ));
    read(&format!("@batch=inner {member}"));
    let case = (kind, parent, ending, member);
    let on_arrival = applied(session.state(), member);
    assert_eq!(on_arrival, live, "on arrival: {case:?}");
    match ending {
        Ending::Close => {
            session.handle_line(now, b":irc.papo.test BATCH -inner");
            if parent.is_some() {
                session.handle_line(now, b":irc.papo.test BATCH -outer");
            }
        }
        Ending::Size => session.handle_line(
            now,
            b"@batch=inner :irc.papo.test NOTICE * :Maintenance complete",
        ),
        Ending::Timeout => session.handle_timeout(now + Duration::from_secs(1)),
        Ending::OpenLimit => {}
    }
    let after = applied(session.state(), member);
    assert_eq!(after, live, "after ending: {case:?}");
}

#[test]
fn live_batch_state_transcript_matrix() {
    for (kind, parent, live) in [
        ("netsplit", None, true),
        ("netjoin", None, true),
        ("chathistory", None, false),
        ("draft/event-playback", None, false),
        ("labeled-response", None, true),
        ("draft/multiline", None, true),
        ("vendor/maintenance", None, true),
        ("netsplit", Some("vendor/maintenance"), true),
        ("netsplit", Some("chathistory"), false),
        ("chathistory", Some("netsplit"), false),
        ("znc.in/playback", None, false),
    ] {
        for ending in [
            Ending::Close,
            Ending::Size,
            Ending::Timeout,
            Ending::OpenLimit,
        ] {
            for &member in MEMBERS {
                transcript(kind, parent, ending, member, live);
            }
        }
    }
}
