//! Transcript coverage for other sources' lines inside the labelled reply to our own send.
//!
//! Ergo answers a labelled `PRIVMSG` to one of its services with a `labeled-response` batch
//! holding our echo followed by the service's replies.

use std::time::Instant;

use papo_core::{Config, Event, RateLimit, Response, Session, TextKind};

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
