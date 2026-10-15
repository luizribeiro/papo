//! Transcript and deadline tests for connection registration.

use std::time::Instant;

use papo_core::{Config, Session};

fn line(session: &mut Session) -> String {
    String::from_utf8(session.poll_transmit().expect("client line"))
        .unwrap()
        .trim_end()
        .to_owned()
}

#[test]
fn sends_registration_in_protocol_order() {
    let now = Instant::now();
    let config = Config::default()
        .nick("emily")
        .username("em")
        .realname("Emily Example")
        .password("correct horse battery staple");
    let mut session = Session::new(config);

    session.handle_connected(now);

    assert_eq!(line(&mut session), "CAP LS 302");
    assert_eq!(line(&mut session), "PASS :correct horse battery staple");
    assert_eq!(line(&mut session), "NICK emily");
    assert_eq!(line(&mut session), "USER em 0 * :Emily Example");
    assert_eq!(session.poll_transmit(), None);
}
