//! Transcript coverage for sends that point at ops-only lines.

use std::time::Instant;

use papo_core::{Config, RateLimit, RequestError, Session};
use papo_proto::{Line, LineRef};

const ERGO_ISUPPORT: &str = ":ergo.test 005 papo CASEMAPPING=ascii CHANTYPES=# PREFIX=(qaohv)~&@%+ STATUSMSG=~&@%+ :are supported by this server";
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
        ("@+draft/reply=$id PRIVMSG #ops :rotated, yes", Some('@')),
        ("@+draft/reply=$id PRIVMSG +#ops :rotated, yes", Some('@')),
        ("@+draft/reply=$id PRIVMSG @#other :rotated", Some('@')),
        ("@+draft/reply=$id PRIVMSG alice :rotated", Some('@')),
        ("@+draft/reply=$id NOTICE #ops :rotated", Some('@')),
        ("@+draft/react=👀;+draft/reply=$id TAGMSG @#ops", None),
        ("@+draft/react=👀;+draft/reply=$id TAGMSG #ops", Some('@')),
        ("@+draft/unreact=👀;+reply=$id TAGMSG #ops", Some('@')),
        (
            "@+reply=4vd2d8wx3tgvc6dsrwkd7hzvsq PRIVMSG #ops :ok then",
            None,
        ),
        ("PRIVMSG #ops :deploy finished", None),
    ] {
        assert_send(OPS_ONLY, "bc3fyyzsmb8ip7aiti6gakzmja", wire, refused);
    }
}
