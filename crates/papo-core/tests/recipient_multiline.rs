//! Transcript coverage for guarded multiline transmission.

use std::time::Instant;

use papo_core::{Account, Config, Event, Session, TextKind};

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

fn discard_events(session: &mut Session) {
    while session.poll_event().is_some() {}
}

fn multiline_session(now: Instant) -> Session {
    let mut session = Session::new(Config::default().nick("papo").wanted_caps([
        "batch",
        "draft/multiline",
        "echo-message",
    ]));
    session.handle_connected(now);
    server(
        &mut session,
        "CAP * LS :batch draft/multiline=max-bytes=4096,max-lines=100 echo-message",
    );
    server(
        &mut session,
        "CAP * ACK :batch draft/multiline echo-message",
    );
    server(
        &mut session,
        ":irc.example 001 papo :Welcome to the network",
    );
    server(&mut session, ":irc.example 422 papo :MOTD File is missing");
    while session.poll_transmit().is_some() {}
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(&mut session, ":alice!~alice@cloak.example JOIN #review");
    server(&mut session, ":mallory!~mallory@users.example JOIN #review");
    discard_events(&mut session);
    session
}

#[test]
fn queued_multiline_direct_message_rewrites_every_target_after_rename() {
    let now = Instant::now();
    let mut session = multiline_session(now);
    let guard = session.__direct_acquire(
        "alice",
        Some("~alice"),
        Some("cloak.example"),
        Account::LoggedOut,
    );
    session
        .send_guarded_text_detailed(
            now,
            TextKind::Privmsg,
            "alice",
            "release ready\nplease review",
            &[],
            Some(guard),
        )
        .unwrap();
    server(&mut session, ":alice!~alice@cloak.example NICK alicia");

    let lines = std::iter::from_fn(|| session.poll_transmit())
        .map(|line| String::from_utf8(line.to_vec()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(lines[0], "BATCH +b0-0 draft/multiline alicia\r\n");
    assert!(lines[1].contains("PRIVMSG alicia :release ready"));
    assert!(lines[2].contains("PRIVMSG alicia :please review"));
    assert_eq!(lines[3], "BATCH -b0-0\r\n");
}

#[test]
fn guarded_multiline_closes_its_batch_after_takeover() {
    let now = Instant::now();
    let mut session = multiline_session(now);
    let guard = session.__direct_acquire(
        "alice",
        Some("~alice"),
        Some("cloak.example"),
        Account::LoggedOut,
    );
    let sent = session
        .send_guarded_text_detailed(
            now,
            TextKind::Notice,
            "alice",
            "release ready\nclassified details",
            &[],
            Some(guard),
        )
        .unwrap();
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"BATCH +b0-0 draft/multiline alice\r\n"
    );
    server(
        &mut session,
        ":alice!~alice@cloak.example QUIT :Client Quit",
    );
    server(&mut session, ":mallory!~mallory@users.example NICK alice");
    discard_events(&mut session);

    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"BATCH -b0-0\r\n"
    );
    assert_eq!(session.poll_transmit(), None);
    assert!(matches!(
        session.poll_event(),
        Some(Event::TransmitRecipientGone { token, .. }) if Some(token) == sent.transmit
    ));
}
