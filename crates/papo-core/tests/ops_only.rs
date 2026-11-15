//! Transcript coverage for sends that point at ops-only lines.

use std::time::Instant;

use papo_core::{Config, RateLimit, RequestError, Session};
use papo_proto::{Line, LineRef};

const ERGO_ISUPPORT: &str = ":ergo.test 005 papo CASEMAPPING=ascii CHANTYPES=# PREFIX=(qaohv)~&@%+ STATUSMSG=~&@%+ :are supported by this server";
const PUBLIC: &str = "@msgid=x8n4qz2kbtqe2nqrwg3hba7m5y;time=2026-11-14T18:02:15.873Z :oppy!~oppy@cloak.example PRIVMSG #ops :standup in five";
const VOICED: &str = "@msgid=k3mz8w2qdvbe4iqx7hgufyn5ta;time=2026-11-14T18:03:40.112Z :vicky!~vicky@cloak.example PRIVMSG +#ops :can someone review?";
const OPS_ONLY: &str = "@msgid=bc3fyyzsmb8ip7aiti6gakzmja;time=2026-11-14T18:02:11.204Z :oppy!~oppy@cloak.example PRIVMSG @#ops :deploy key rotated?";

fn ergo(now: Instant, lines: &[&str]) -> Session {
    let mut config = Config::default();
    config.rate_limit = RateLimit::unlimited();
    let mut session = Session::new(config);
    session.handle_connected(now);
    let welcome = [
        ":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo",
        ERGO_ISUPPORT,
        ":ergo.test 422 papo :MOTD File is missing",
    ];
    for line in welcome.iter().chain(lines) {
        session.handle_line(now, line.as_bytes());
    }
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
    session
}

fn line(wire: &str) -> Line {
    LineRef::parse(wire).expect("test line parses").to_owned()
}

fn refused_as(result: &Result<(), RequestError>, symbol: char) -> bool {
    matches!(result, Err(RequestError::WiderThanReferenced { referenced, .. })
        if referenced.symbol() == symbol)
}

/// Sends `wire`, with `$id` standing for the received line's msgid, after the server played
/// `received`; then checks that it was refused for `refused`'s sign or went out unchanged.
fn assert_send(received: &str, id: &str, wire: &str, refused: Option<char>) {
    let now = Instant::now();
    let mut session = ergo(now, &[received]);
    let wire = wire.replace("$id", id);
    let result = session.send(now, line(&wire));
    let sent = session
        .poll_transmit()
        .map(|sent| String::from_utf8_lossy(&sent).trim_end().to_owned());
    if let Some(symbol) = refused {
        assert!(refused_as(&result, symbol), "{wire}: {result:?}");
        assert_eq!(sent, None, "{wire}");
    } else {
        assert_eq!(result, Ok(()), "{wire}");
        assert_eq!(sent.as_deref(), Some(wire.as_str()));
    }
}

#[test]
fn a_send_pointing_at_an_ops_only_line_may_only_narrow_its_audience() {
    for (wire, refused) in [
        ("@+draft/reply=$id PRIVMSG @#ops :rotated, yes", None),
        ("@+reply=$id PRIVMSG @#OPS :rotated, yes", None),
        ("@+reply=$id PRIVMSG ~#ops :owners: rotated", None),
        ("@+reply=$id PRIVMSG &#ops :admins: rotated", None),
        ("@+reply=$id PRIVMSG %#ops :halfops: rotated", Some('@')),
        ("@+draft/reply=$id PRIVMSG #ops :rotated, yes", Some('@')),
        ("@+draft/reply=$id PRIVMSG +#ops :rotated, yes", Some('@')),
        ("@+draft/reply=$id PRIVMSG @#other :rotated", Some('@')),
        ("@+draft/reply=$id PRIVMSG alice :rotated", Some('@')),
        ("@+draft/reply=$id NOTICE #ops :rotated", Some('@')),
        ("@+draft/react=👀;+draft/reply=$id TAGMSG @#ops", None),
        ("@+draft/react=👀;+draft/reply=$id TAGMSG #ops", Some('@')),
        ("@+draft/unreact=👀;+reply=$id TAGMSG #ops", Some('@')),
        ("REDACT @#ops $id :wrong channel", None),
        ("REDACT #ops $id :wrong channel", Some('@')),
        ("REDACT #ops $id", Some('@')),
        (
            "@+reply=4vd2d8wx3tgvc6dsrwkd7hzvsq PRIVMSG #ops :ok then",
            None,
        ),
        ("PRIVMSG #ops :deploy finished", None),
    ] {
        assert_send(OPS_ONLY, "bc3fyyzsmb8ip7aiti6gakzmja", wire, refused);
    }
}

fn ops_only_line(id: &str) -> String {
    format!("@msgid={id} :oppy!~oppy@cloak.example PRIVMSG @#ops :rotating key {id}")
}

#[test]
fn the_oldest_of_1025_ops_only_ids_is_forgotten() {
    let ids: Vec<String> = (0..1025)
        .map(|n| format!("bc3fyyzsmb8ip7aiti6g{n:06}"))
        .collect();
    let mut received: Vec<String> = ids.iter().map(|id| ops_only_line(id)).collect();
    received.insert(1, ops_only_line(&ids[1]));
    let received: Vec<&str> = received.iter().map(String::as_str).collect();
    let now = Instant::now();
    let mut session = ergo(now, &received);

    let forgotten = format!("@+draft/reply={} PRIVMSG #ops :rotated, yes", ids[0]);
    assert_eq!(session.send(now, line(&forgotten)), Ok(()));
    for id in [&ids[1], &ids[1024]] {
        let wider = format!("@+draft/reply={id} PRIVMSG #ops :rotated, yes");
        assert!(refused_as(&session.send(now, line(&wider)), '@'), "{id}");
    }
}

#[test]
fn a_voiced_line_may_be_answered_to_any_higher_rank() {
    for (wire, refused) in [
        ("@+reply=$id PRIVMSG +#ops :on it", None),
        ("@+reply=$id PRIVMSG %#ops :halfops: on it", None),
        ("@+reply=$id PRIVMSG @#ops :ops: on it", None),
        ("@+reply=$id PRIVMSG #ops :on it", Some('+')),
    ] {
        assert_send(VOICED, "k3mz8w2qdvbe4iqx7hgufyn5ta", wire, refused);
    }
}

#[test]
fn a_public_line_may_be_answered_publicly_or_to_its_ops() {
    for wire in [
        "@+draft/reply=$id PRIVMSG @#ops :ops: about standup",
        "@+draft/reply=$id PRIVMSG #ops :joining now",
    ] {
        assert_send(PUBLIC, "x8n4qz2kbtqe2nqrwg3hba7m5y", wire, None);
    }
}

#[test]
fn a_sign_missing_from_prefix_matches_only_itself() {
    let now = Instant::now();
    let mut session = ergo(
        now,
        &[
            ":irc.example 005 papo PREFIX=(ov)@+ STATUSMSG=@+% :are supported by this server",
            "@msgid=Zq7Hk2Rp :hops!~h@cloak.example PRIVMSG %#ops :halfops, check the ban list",
        ],
    );
    let same = "@+draft/reply=Zq7Hk2Rp PRIVMSG %#ops :checked, two stale bans";
    assert_eq!(session.send(now, line(same)), Ok(()));
    let ranked = "@+draft/reply=Zq7Hk2Rp PRIVMSG @#ops :checked, two stale bans";
    assert!(refused_as(&session.send(now, line(ranked)), '%'));
}
