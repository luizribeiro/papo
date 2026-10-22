#![no_main]

use libfuzzer_sys::fuzz_target;
use papo_core::{Config, RateLimit, Session, TextKind};
use papo_proto::{Ctcp, LineRef};
use web_time::Instant;

const TARGET: &str = "#papo";

fn session(now: Instant, multiline: bool) -> Session {
    let caps = multiline.then_some(["batch", "draft/multiline"]);
    let mut session = Session::new(
        Config::default()
            .wanted_caps(caps.into_iter().flatten())
            .rate_limit(RateLimit::unlimited()),
    );
    session.handle_connected(now);
    if multiline {
        session.handle_line(
            now,
            b"CAP * LS :batch draft/multiline=max-bytes=4096,max-lines=100",
        );
        session.handle_line(now, b"CAP * ACK :batch draft/multiline");
    } else {
        session.handle_line(now, b"CAP * LS :");
    }
    while session.poll_transmit().is_some() {}
    session
}

fuzz_target!(|input: &[u8]| {
    let Some((&flags, input)) = input.split_first() else {
        return;
    };
    let multiline = flags & 1 != 0;
    let action = flags & 2 != 0;
    let kind = if flags & 4 != 0 {
        TextKind::Notice
    } else {
        TextKind::Privmsg
    };
    let input = input.strip_suffix(b"\n").unwrap_or(input);
    let mut text = String::from_utf8_lossy(input).into_owned();
    if flags & 8 != 0 {
        text.push(' ');
    }
    let now = Instant::now();
    let mut session = session(now, multiline);
    let result = if action {
        session.send_action_detailed(now, TARGET, &text, &[])
    } else {
        session.send_text_detailed(now, kind, TARGET, &text, &[])
    };
    if result.is_err() {
        assert!(session.poll_transmit().is_none());
        return;
    }

    while let Some(bytes) = session.poll_transmit() {
        assert!(bytes.ends_with(b"\r\n"));
        assert!(bytes.len() <= 512);
        let wire = String::from_utf8(bytes.to_vec()).unwrap();
        let line = LineRef::parse(&wire).unwrap();
        if line.command_is("PRIVMSG") || line.command_is("NOTICE") {
            assert_eq!(line.param(0), Some(TARGET));
            let body = line.param(1).unwrap();
            if action && !multiline {
                assert_eq!(Ctcp::parse(body).map(|ctcp| ctcp.command), Some("ACTION"));
            } else if !action {
                assert!(!body.starts_with('\x01'));
            }
        }
    }
});
