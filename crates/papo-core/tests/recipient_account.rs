//! Transcript coverage: a direct recipient is never resolved through another user's account.

use std::time::Instant;

use papo_core::{Account, Config, Event, Session};
use papo_proto::Line;

const RELEASE_TOKEN: &str = "alice only: the deploy token is hunter2";

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

fn discard_events(session: &mut Session) {
    while session.poll_event().is_some() {}
}

fn send_to_alice_after(lines: &[&str]) -> (Option<Vec<u8>>, Vec<Event>) {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, ":papo!~papo@localhost JOIN #release * :Papo");
    server(
        &mut session,
        ":alice!~alice@cloak.example JOIN #release alice-account :Alice",
    );
    let account = Account::LoggedIn(session.state().casemapping().account("alice-account"));
    let guard = session
        .state()
        .__capture_recipient(session.state().casemapping().nick("alice"), account)
        .unwrap();
    for line in lines {
        server(&mut session, line);
    }
    discard_events(&mut session);
    session
        .send_guarded_tracked(now, Line::privmsg("alice", RELEASE_TOKEN), Some(guard))
        .unwrap();
    let sent = session.poll_transmit().map(|line| line.as_ref().to_vec());
    (sent, std::iter::from_fn(|| session.poll_event()).collect())
}

fn assert_refused(lines: &[&str]) {
    let (sent, events) = send_to_alice_after(lines);
    assert!(
        sent.is_none(),
        "{:?}",
        sent.as_deref().map(String::from_utf8_lossy)
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::TransmitRecipientGone { .. })),
        "{events:?}"
    );
}

#[test]
fn departed_handle_is_not_retargeted_to_another_nick_on_the_account() {
    assert_refused(&[
        ":relaybot!~relay@bots.example JOIN #release alice-account :Shared relay bot",
        ":alice!~alice@cloak.example PART #release :bye",
    ]);
}

#[test]
fn departed_handle_is_not_retargeted_among_several_nicks_on_the_account() {
    assert_refused(&[
        ":relaybot!~relay@bots.example JOIN #release alice-account :Shared relay bot",
        ":alice-phone!~alice@mobile.example JOIN #release alice-account :Alice (phone)",
        ":alice-laptop!~alice@laptop.example JOIN #release alice-account :Alice (laptop)",
        ":alice!~alice@cloak.example PART #release :bye",
    ]);
}

#[test]
fn quit_handle_is_not_retargeted_to_another_nick_on_the_account() {
    assert_refused(&[
        ":relaybot!~relay@bots.example JOIN #release alice-account :Shared relay bot",
        ":alice!~alice@cloak.example QUIT :Quit: rebooting",
    ]);
}

#[test]
fn quit_handle_is_not_retargeted_to_a_new_session_with_its_nick_and_account() {
    assert_refused(&[
        ":alice!~alice@cloak.example QUIT :Quit: rebooting",
        ":alice!~alice@laptop.example JOIN #release alice-account :Alice",
    ]);
}

#[test]
fn departed_handle_without_another_nick_on_the_account_is_refused() {
    assert_refused(&[":alice!~alice@cloak.example PART #release :bye"]);
}

#[test]
fn present_handle_is_sent_to_its_own_nick_beside_others_on_the_account() {
    let (sent, events) = send_to_alice_after(&[
        ":relaybot!~relay@bots.example JOIN #release alice-account :Shared relay bot",
        ":alice-phone!~alice@mobile.example JOIN #release alice-account :Alice (phone)",
    ]);
    assert_eq!(
        sent.as_deref().map(String::from_utf8_lossy),
        Some(format!("PRIVMSG alice :{RELEASE_TOKEN}\r\n").into())
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::TransmitRecipientGone { .. })),
        "{events:?}"
    );
}
