//! Regression coverage for proof-backed person sends.

use std::time::Instant;

use papo_core::{Account, Config, Event, PersonCommand, Proven, Session, TextKind};

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

fn discard_events(session: &mut Session) {
    while session.poll_event().is_some() {}
}

fn shared_users() -> Session {
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, ":papo!p@localhost JOIN #review");
    server(
        &mut session,
        ":alice!a@alice.example JOIN #review alice-account :Alice",
    );
    server(
        &mut session,
        ":mallory!m@mallory.example JOIN #review mallory-account :Mallory",
    );
    discard_events(&mut session);
    session
}

fn event_proof(session: &mut Session) -> Proven {
    server(session, ":alice!a@alice.example PRIVMSG #review :ready");
    let token = loop {
        match session.poll_event() {
            Some(Event::Line(line)) if line.as_ref().command_is("PRIVMSG") => {
                break line.__reply_token();
            }
            Some(_) => {}
            None => panic!("message event is available"),
        }
    };
    session
        .state()
        .__prove_event_user(
            token,
            session.state().casemapping().nick("alice"),
            Account::LoggedIn(session.state().casemapping().account("alice-account")),
        )
        .unwrap()
}

#[test]
fn proven_event_user_follows_nick() {
    let mut session = shared_users();
    let proof = event_proof(&mut session);
    session
        .send_person_text_detailed(
            Instant::now(),
            proof,
            TextKind::Privmsg,
            "release approved",
            &[],
        )
        .unwrap();
    server(&mut session, ":alice!a@alice.example NICK alicia");

    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"PRIVMSG alicia :release approved\r\n"
    );
}

#[test]
fn proven_user_refuses_departed_identity() {
    let mut session = shared_users();
    let proof = event_proof(&mut session);
    let token = session
        .send_person_text_detailed(Instant::now(), proof, TextKind::Notice, "classified", &[])
        .unwrap()
        .transmit
        .unwrap();
    server(&mut session, ":alice!a@alice.example QUIT :Client Quit");
    server(&mut session, ":mallory!m@mallory.example NICK alice");
    discard_events(&mut session);

    assert_eq!(session.poll_transmit(), None);
    assert!(matches!(
        session.poll_event(),
        Some(Event::TransmitRecipientGone { token: gone, .. }) if gone == token
    ));
}

#[test]
fn current_name_targets_current_holder() {
    let mut session = shared_users();
    let nick = session.state().casemapping().nick("alice");
    let proof = session.__prove_current_name(nick).unwrap();
    session
        .send_person_text_detailed(
            Instant::now(),
            proof,
            TextKind::Privmsg,
            "please review",
            &[],
        )
        .unwrap();
    server(&mut session, ":alice!a@alice.example QUIT :Client Quit");
    server(&mut session, ":mallory!m@mallory.example NICK alice");

    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"PRIVMSG alice :please review\r\n"
    );
}

#[test]
fn queued_send_rewrites_renamed_recipient() {
    let mut session = shared_users();
    let proof = event_proof(&mut session);
    let command = PersonCommand::__new(papo_proto::Line::tagmsg("alice")).unwrap();
    session
        .send_person_tracked(Instant::now(), proof.clone(), command)
        .unwrap();
    server(&mut session, ":alice!a@alice.example NICK alicia");

    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"TAGMSG alicia\r\n"
    );

    session
        .send_person_action_detailed(Instant::now(), proof, "waves", &[])
        .unwrap();
    server(&mut session, ":alicia!a@alice.example NICK alice-new");
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"PRIVMSG alice-new :\x01ACTION waves\x01\r\n"
    );
}
