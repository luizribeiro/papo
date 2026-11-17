//! Transcript coverage for other sources' lines inside the labelled reply to our own send.
//!
//! Ergo answers a labelled `PRIVMSG` to one of its services with a `labeled-response` batch
//! holding our echo followed by the service's replies.

use std::time::Instant;

use papo_core::{Config, Event, RateLimit, Response, Session, TextKind};
use papo_proto::{Command, Line};

const CAPS: &str = "batch echo-message labeled-response message-tags server-time";

fn ergo(now: Instant) -> Session {
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited()),
    );
    session.handle_connected(now);
    for line in [
        format!(":ergo.test CAP * LS :{CAPS}"),
        format!(":ergo.test CAP * ACK :{CAPS}"),
        ":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo".into(),
        ":ergo.test 005 papo CASEMAPPING=ascii CHANTYPES=# :are supported by this server".into(),
        ":ergo.test 422 papo :MOTD File is missing".into(),
    ] {
        session.handle_line(now, line.as_bytes());
    }
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
    session
}

fn play(session: &mut Session, now: Instant, lines: &[&str]) -> Vec<Event> {
    for line in lines {
        session.handle_line(now, line.as_bytes());
    }
    std::iter::from_fn(|| session.poll_event()).collect()
}

fn reported_lines(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Line(received) => {
                let line = received.to_string();
                Some(match line.split_once(" :") {
                    Some((tags, rest)) if tags.starts_with('@') => format!(":{rest}"),
                    _ => line,
                })
            }
            _ => None,
        })
        .collect()
}

fn response(events: &[Event]) -> &Response {
    let mut responses = events.iter().filter_map(|event| match event {
        Event::Response(response) => Some(response),
        _ => None,
    });
    let response = responses.next().expect("the send is confirmed");
    assert!(responses.next().is_none(), "{events:?}");
    response
}

const NICKSERV_INFO: [&str; 4] = [
    "@label=0 :ergo.test BATCH +3 labeled-response",
    "@msgid=by3fzn7wkesjv28zvis9h8b7d6;time=2026-11-17T18:45:08.001Z;batch=3 :papo!~u@npm4sa48pgvdy.irc PRIVMSG NickServ :INFO",
    "@time=2026-11-17T18:45:08.001Z;batch=3 :NickServ!NickServ@localhost NOTICE papo :You're not logged into an account",
    ":ergo.test BATCH -3",
];

#[test]
fn a_service_reply_inside_our_send_batch_is_reported() {
    let now = Instant::now();
    let mut session = ergo(now);
    session
        .send_text(now, TextKind::Privmsg, "NickServ", "INFO", &[])
        .unwrap();
    let events = play(&mut session, now, &NICKSERV_INFO);

    assert_eq!(
        reported_lines(&events),
        [":NickServ!NickServ@localhost NOTICE papo :You're not logged into an account"]
    );
    let sent = response(&events)
        .sent()
        .expect("the echo confirms the send");
    assert_eq!(sent.msgid, "by3fzn7wkesjv28zvis9h8b7d6");
    assert!(
        matches!(events.as_slice(), [Event::Response(_), Event::Line(_)]),
        "the reply follows the confirmation: {events:?}"
    );
}

#[test]
fn every_line_of_a_service_help_reply_is_reported_in_order() {
    let now = Instant::now();
    let mut session = ergo(now);
    session
        .send_text(now, TextKind::Privmsg, "ChanServ", "HELP", &[])
        .unwrap();
    let help = [
        "*** ChanServ HELP ***",
        "ChanServ lets you register and manage channels.",
        "",
        "To see in-depth help for a specific command, try:",
        "    /msg ChanServ HELP <command>",
        "*** End of ChanServ HELP ***",
    ];
    let mut lines = vec![
        "@label=0 :ergo.test BATCH +2 labeled-response".to_owned(),
        "@msgid=rtuqk455g9z76bwejdc8egynjn;time=2026-11-17T18:45:06.500Z;batch=2 :papo!~u@npm4sa48pgvdy.irc PRIVMSG ChanServ :HELP".to_owned(),
    ];
    lines.extend(help.iter().map(|text| {
        format!("@time=2026-11-17T18:45:06.500Z;batch=2 :ChanServ!ChanServ@localhost NOTICE papo :{text}")
    }));
    lines.push(":ergo.test BATCH -2".into());
    let lines = lines.iter().map(String::as_str).collect::<Vec<_>>();
    let events = play(&mut session, now, &lines);

    let expected = help
        .iter()
        .map(|text| format!(":ChanServ!ChanServ@localhost NOTICE papo :{text}"))
        .collect::<Vec<_>>();
    assert_eq!(reported_lines(&events), expected);
    assert!(response(&events).sent().is_some());
}

#[test]
fn a_batch_with_only_our_echo_reports_nothing_else() {
    let now = Instant::now();
    let mut session = ergo(now);
    session
        .send_text(now, TextKind::Privmsg, "NickServ", "INFO", &[])
        .unwrap();
    let events = play(
        &mut session,
        now,
        &[NICKSERV_INFO[0], NICKSERV_INFO[1], NICKSERV_INFO[3]],
    );

    assert_eq!(reported_lines(&events), Vec::<String>::new());
    assert!(response(&events).sent().is_some());
    assert_eq!(events.len(), 1, "{events:?}");
}

#[test]
fn a_notice_or_action_to_a_service_is_confirmed_by_its_lone_echo() {
    let now = Instant::now();
    for (action, echo) in [
        (
            false,
            "@msgid=hvtikm5shh7amyg4pzfgwbg8qs;time=2026-11-17T18:45:03.498Z;label=0 :papo!~u@npm4sa48pgvdy.irc NOTICE NickServ :HELP",
        ),
        (
            true,
            "@msgid=svk9smy7773jsffyzyhqymd54s;time=2026-11-17T18:45:04.999Z;label=0 :papo!~u@npm4sa48pgvdy.irc PRIVMSG NickServ :\u{1}ACTION waves\u{1}",
        ),
    ] {
        let mut session = ergo(now);
        if action {
            session.send_action_detailed(now, "NickServ", "waves", &[])
        } else {
            session.send_text_detailed(now, TextKind::Notice, "NickServ", "HELP", &[])
        }
        .unwrap();
        let events = play(&mut session, now, &[echo]);

        assert_eq!(reported_lines(&events), Vec::<String>::new(), "{echo}");
        assert!(response(&events).sent().is_some(), "{echo}");
    }
}

#[test]
fn a_notice_or_action_echo_in_a_batch_is_ours_and_the_service_reply_is_not() {
    let now = Instant::now();
    for (action, echo) in [
        (
            false,
            "@msgid=hvtikm5shh7amyg4pzfgwbg8qs;time=2026-11-17T18:45:03.498Z;batch=3 :papo!~u@npm4sa48pgvdy.irc NOTICE NickServ :INFO",
        ),
        (
            true,
            "@msgid=svk9smy7773jsffyzyhqymd54s;time=2026-11-17T18:45:04.999Z;batch=3 :papo!~u@npm4sa48pgvdy.irc PRIVMSG NickServ :\u{1}ACTION waves\u{1}",
        ),
    ] {
        let mut session = ergo(now);
        if action {
            session.send_action_detailed(now, "NickServ", "waves", &[])
        } else {
            session.send_text_detailed(now, TextKind::Notice, "NickServ", "INFO", &[])
        }
        .unwrap();
        let events = play(
            &mut session,
            now,
            &[NICKSERV_INFO[0], echo, NICKSERV_INFO[2], NICKSERV_INFO[3]],
        );

        assert_eq!(
            reported_lines(&events),
            [":NickServ!NickServ@localhost NOTICE papo :You're not logged into an account"],
            "{echo}"
        );
        assert!(response(&events).sent().is_some(), "{echo}");
    }
}

#[test]
fn an_ack_only_reply_confirms_without_reporting_anything() {
    let now = Instant::now();
    let mut session = ergo(now);
    let id = session
        .request_silent(
            now,
            Line::new("TAGMSG")
                .with_tag("+typing", "active")
                .param("NickServ"),
        )
        .unwrap();
    let events = play(
        &mut session,
        now,
        &["@label=0;time=2026-11-17T18:45:11.005Z :ergo.test ACK"],
    );

    assert_eq!(events.len(), 1, "{events:?}");
    let response = response(&events);
    assert_eq!(response.id, id);
    assert!(response.lines.is_empty());
    assert!(response.error().is_none());
}

#[test]
fn the_response_still_carries_every_line_of_the_batch() {
    let now = Instant::now();
    let mut session = ergo(now);
    session
        .send_text(now, TextKind::Privmsg, "NickServ", "INFO", &[])
        .unwrap();
    let events = play(&mut session, now, &NICKSERV_INFO);

    let commands = response(&events)
        .lines
        .iter()
        .map(|line| line.as_ref().command())
        .collect::<Vec<_>>();
    assert_eq!(commands, [Command::Privmsg, Command::Notice]);
}

fn joined(now: Instant) -> Session {
    let mut session = ergo(now);
    play(
        &mut session,
        now,
        &[
            "@msgid=nvdu3cib6mfsqazvdvv8u976ma;time=2026-11-17T18:52:56.692Z :papo!~u@npm4sa48pgvdy.irc JOIN #papo",
            ":ergo.test 353 papo = #papo :@papo bob",
            ":ergo.test 366 papo #papo :End of NAMES list",
        ],
    );
    session
}

#[test]
fn what_a_service_command_changes_is_reported() {
    let now = Instant::now();
    let mut session = joined(now);
    session
        .send_text(now, TextKind::Privmsg, "NickServ", "REGISTER hunter2", &[])
        .unwrap();
    let account = play(
        &mut session,
        now,
        &[
            "@label=0 :ergo.test BATCH +1 labeled-response",
            "@msgid=65ybgj23gmak7trgzvkbti4f5i;time=2026-11-17T18:46:56.875Z;batch=1 :papo!~u@npm4sa48pgvdy.irc PRIVMSG NickServ :REGISTER hunter2",
            "@time=2026-11-17T18:46:56.877Z;batch=1 :NickServ!NickServ@localhost NOTICE papo :Account created",
            "@time=2026-11-17T18:46:56.877Z;batch=1 :NickServ!NickServ@localhost NOTICE papo :You're now logged in as papo",
            "@time=2026-11-17T18:46:56.877Z;batch=1 :papo!~u@npm4sa48pgvdy.irc ACCOUNT papo",
            ":ergo.test BATCH -1",
        ],
    );
    session
        .send_text(now, TextKind::Privmsg, "ChanServ", "REGISTER #papo", &[])
        .unwrap();
    let channel = play(
        &mut session,
        now,
        &[
            "@label=1 :ergo.test BATCH +6 labeled-response",
            "@msgid=fkjf3bbnmbmpmtdjzih2i3akt2;time=2026-11-17T18:47:07.388Z;batch=6 :papo!~u@npm4sa48pgvdy.irc PRIVMSG ChanServ :REGISTER #papo",
            "@time=2026-11-17T18:47:07.388Z;batch=6 :ChanServ!ChanServ@localhost NOTICE papo :Channel #papo successfully registered",
            "@msgid=fk3vmm6fj8ek69ryjcu7694yz2;time=2026-11-17T18:47:07.388Z;batch=6 :ChanServ!ChanServ@localhost MODE #papo +q papo",
            ":ergo.test BATCH -6",
        ],
    );

    assert_eq!(
        reported_lines(&account),
        [
            ":NickServ!NickServ@localhost NOTICE papo :Account created",
            ":NickServ!NickServ@localhost NOTICE papo :You're now logged in as papo",
        ]
    );
    assert!(
        account
            .iter()
            .any(|event| matches!(event, Event::AccountChanged { .. })),
        "{account:?}"
    );
    assert_eq!(
        reported_lines(&channel),
        [":ChanServ!ChanServ@localhost NOTICE papo :Channel #papo successfully registered"]
    );
    assert!(
        channel
            .iter()
            .any(|event| matches!(event, Event::ModeChanged { .. })),
        "{channel:?}"
    );
    assert!(response(&account).sent().is_some() && response(&channel).sent().is_some());
}
