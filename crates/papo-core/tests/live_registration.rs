//! End-to-end registration against the pinned Ergo test server.
#![cfg(any(
    not(target_os = "wasi"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

mod support;

use std::{net::TcpStream, time::Instant};

use papo_core::{Config, Event, RateLimit, Session};
use papo_proto::Casemapping;
use papo_test_server::{from_env, unique};
use support::{drive_until, within};

#[test]
fn registers_with_live_ergo() {
    let server = from_env();
    let mut stream = TcpStream::connect(server.plaintext_addr()).unwrap();
    let nick = unique("papo-registration");
    let mut session = Session::new(
        Config::default()
            .nick(nick.clone())
            .username("papo")
            .realname("papo live registration test")
            .rate_limit(RateLimit::unlimited()),
    );
    session.handle_connected(Instant::now());

    let event = within(&mut stream, |stream| {
        drive_until(&mut session, stream, 64, |event| {
            matches!(event, Event::Connected(_) | Event::Close { .. })
        })
    })
    .expect("registration did not complete");
    if let Event::Close { reason, .. } = event {
        panic!("session closed: {reason}");
    }
    assert_eq!(session.nick(), nick);
    assert!(session.caps().contains_key("message-tags"));
    assert!(session.caps().contains_key("server-time"));
    assert_eq!(session.isupport().casemapping(), Casemapping::Ascii);
}
