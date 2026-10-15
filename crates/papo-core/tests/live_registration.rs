//! End-to-end registration against the pinned Ergo test server.
#![cfg(any(
    not(target_os = "wasi"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use std::{
    io::{Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

use papo_core::{Config, Event, RateLimit, Session};
use papo_proto::Casemapping;
use papo_test_server::{from_env, unique};

#[test]
fn registers_with_live_ergo() {
    let server = from_env();
    let mut stream = TcpStream::connect(server.plaintext_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let nick = unique("papo-registration");
    let mut session = Session::new(
        Config::default()
            .nick(nick.clone())
            .username("papo")
            .realname("papo live registration test")
            .rate_limit(RateLimit::unlimited()),
    );
    session.handle_connected(Instant::now());

    let mut registered = false;
    for _ in 0..64 {
        while let Some(bytes) = session.poll_transmit() {
            stream.write_all(&bytes).unwrap();
        }
        while let Some(event) = session.poll_event() {
            match event {
                Event::Registered(_) => registered = true,
                Event::Close { reason } => panic!("session closed: {reason}"),
                _ => {}
            }
        }
        if registered {
            break;
        }
        let mut bytes = [0; 16 * 1024];
        let count = stream.read(&mut bytes).unwrap();
        assert_ne!(count, 0, "Ergo closed before registration");
        session.handle_bytes(Instant::now(), &bytes[..count]);
    }

    assert!(registered, "registration did not complete");
    assert_eq!(session.nick(), nick);
    assert!(session.caps().contains_key("message-tags"));
    assert!(session.caps().contains_key("server-time"));
    assert_eq!(session.isupport().casemapping(), Casemapping::Ascii);
}
