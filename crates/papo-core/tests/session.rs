//! Tests for the public session queue surface.

use std::time::{Duration, Instant};

use papo_core::sts::ConnectionSecurity;
use papo_core::{Config, DEFAULT_WANTED_CAPS, RateLimit, Session};
use papo_proto::{BuildError, Message};

#[test]
fn config_builders_set_registration_identity_and_timeout() {
    let config = Config::default()
        .nick("emily")
        .username("em")
        .realname("Emily Example")
        .password("correct horse battery staple")
        .registration_timeout(Duration::from_secs(23))
        .ping_interval(Duration::from_secs(41))
        .ping_timeout(Duration::from_secs(82))
        .rate_limit(RateLimit::new(7, Duration::from_millis(250)))
        .max_channels(17)
        .max_members_per_channel(23)
        .connection_security(ConnectionSecurity::Secure)
        .host("irc.example.net");

    assert_eq!(config.nick, "emily");
    assert_eq!(config.username, "em");
    assert_eq!(config.realname, "Emily Example");
    assert_eq!(
        config.password.as_deref(),
        Some("correct horse battery staple")
    );
    assert_eq!(config.connection_security, ConnectionSecurity::Secure);
    assert_eq!(config.host, "irc.example.net");
    assert_eq!(config.registration_timeout, Duration::from_secs(23));
    assert_eq!(config.ping_interval, Duration::from_secs(41));
    assert_eq!(config.ping_timeout, Duration::from_secs(82));
    assert_eq!(config.max_channels, 17);
    assert_eq!(config.max_members_per_channel, 23);
    assert_eq!(
        config.rate_limit,
        RateLimit::new(7, Duration::from_millis(250))
    );
    assert_eq!(
        config.wanted_caps,
        DEFAULT_WANTED_CAPS
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    );

    let config = Config::default().wanted_caps(["server-time", "batch"]);
    assert_eq!(config.wanted_caps, ["server-time", "batch"]);

    let config = Config::default().alt_nicks(["emily_", "emily__"]);
    assert_eq!(config.alt_nicks, ["emily_", "emily__"]);

    let config = Config::default();
    assert_eq!(config.max_channels, 10_000);
    assert_eq!(config.max_members_per_channel, 100_000);
}

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

#[test]
fn state_generation_changes_only_with_snapshot_state() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("emily"));
    assert_eq!(session.state_generation(), 0);

    session.handle_connected(now);
    assert_eq!(session.state_generation(), 1);
    session.handle_line(now, b":irc.example NOTICE emily :maintenance soon");
    assert_eq!(session.state_generation(), 1);

    session.handle_line(
        now,
        b":irc.example 005 emily CASEMAPPING=ascii :are supported",
    );
    assert_eq!(session.state_generation(), 2);
    session.handle_line(
        now,
        b":irc.example 005 emily CASEMAPPING=ascii :are supported",
    );
    assert_eq!(session.state_generation(), 2);

    session.handle_line(now, b":emily!em@localhost JOIN #papo");
    assert_eq!(session.state_generation(), 3);
}
