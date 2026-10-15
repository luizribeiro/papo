//! Tests for the public session queue surface.

use std::time::Instant;

use papo_core::{Config, Session};
use papo_proto::{BuildError, Message};

#[test]
fn sends_crlf_terminated_messages_in_order() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.send(now, Message::new("NICK", ["tester"])).unwrap();
    session.send(now, Message::join("#papo")).unwrap();

    assert_eq!(session.poll_transmit(), Some(b"NICK tester\r\n".to_vec()));
    assert_eq!(session.poll_transmit(), Some(b"JOIN #papo\r\n".to_vec()));
    assert_eq!(session.poll_transmit(), None);
    assert_eq!(session.poll_event(), None);
    assert_eq!(session.poll_timeout(), None);
}

#[test]
fn disconnect_drops_queued_transmissions() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.send(now, Message::new("NICK", ["tester"])).unwrap();
    session.handle_disconnected(now);
    assert_eq!(session.poll_transmit(), None);
}

#[test]
fn rejects_an_invalid_message_before_queueing_a_valid_one() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    assert_eq!(
        session.send(now, Message::new("", ["tester"])),
        Err(BuildError::InvalidCommand)
    );
    assert_eq!(session.send(now, Message::join("#papo")), Ok(()));
    assert_eq!(session.poll_transmit(), Some(b"JOIN #papo\r\n".to_vec()));
    assert_eq!(session.poll_transmit(), None);
}
