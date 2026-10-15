//! Transcript tests for session-owned ISUPPORT state.

use std::time::Instant;

use papo_core::{Config, Event, Session};
use papo_proto::Advertised;

fn discard_registration(session: &mut Session) {
    for _ in 0..3 {
        session.poll_transmit();
    }
}

#[test]
fn applies_multiple_005_lines_and_enforces_utf8only() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_connected(now);
    discard_registration(&mut session);

    session.handle_line(
        now,
        b":ergo.test 005 papo CASEMAPPING=ascii NICKLEN=32 :are supported by this server",
    );
    session.handle_line(
        now,
        b":ergo.test 005 papo NETWORK=Ergo UTF8ONLY :are supported by this server",
    );
    assert_eq!(session.isupport().nicklen(), Some(32));
    assert_eq!(session.isupport().network(), Some("Ergo"));
    assert_eq!(session.isupport().get("UTF8ONLY"), Advertised::Bare);

    session.handle_line(now, b":ergo.test NOTICE papo :caf\xe9");
    assert_eq!(
        session.poll_event(),
        Some(Event::Malformed {
            line: b":ergo.test NOTICE papo :caf\xe9".to_vec()
        })
    );
}

#[test]
fn removing_utf8only_restores_the_configured_decoding_policy() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_connected(now);
    discard_registration(&mut session);
    session.handle_line(
        now,
        b":soju.example 005 papo UTF8ONLY :are supported by this server",
    );
    session.handle_line(
        now,
        b":soju.example 005 papo -UTF8ONLY :are supported by this server",
    );
    session.handle_line(now, b":soju.example NOTICE papo :caf\xe9");

    assert!(matches!(session.poll_event(), Some(Event::Message(_))));
}
