//! Tests for the public session queue surface.

use std::time::{Duration, Instant};

use papo_core::sts::ConnectionSecurity;
use papo_core::{Config, DEFAULT_WANTED_CAPS, Event, RateLimit, RequestError, Session, TextKind};
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
        .max_open_batches(3)
        .max_batch_lines(5)
        .batch_timeout(Duration::from_secs(7))
        .request_timeout(Duration::from_secs(11))
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
    assert_eq!(config.max_open_batches, 3);
    assert_eq!(config.max_batch_lines, 5);
    assert_eq!(config.batch_timeout, Duration::from_secs(7));
    assert_eq!(config.request_timeout, Duration::from_secs(11));
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
    assert_eq!(config.max_open_batches, 64);
    assert_eq!(config.max_batch_lines, 10_000);
    assert_eq!(config.batch_timeout, Duration::from_secs(120));
    assert_eq!(config.request_timeout, Duration::from_secs(30));
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

#[test]
fn annotates_echoes_with_the_active_casemapping() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("Papo[bot]"));
    session.handle_line(now, b":papo{bot}!~papo@localhost PRIVMSG #papo :sent");
    session.handle_line(now, b":alice!~a@localhost PRIVMSG #papo :received");

    let Some(Event::Message(echo)) = session.poll_event() else {
        panic!("expected echoed message")
    };
    let Some(Event::Message(received)) = session.poll_event() else {
        panic!("expected received message")
    };
    assert!(echo.is_echo());
    assert!(!received.is_echo());
}

#[test]
fn labels_distinguish_our_echo_from_another_session() {
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .nick("papo")
            .wanted_caps(["echo-message", "labeled-response"]),
    );
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :echo-message labeled-response");
    session.handle_line(now, b"CAP * ACK :echo-message labeled-response");
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
    session
        .send_text(now, TextKind::Privmsg, "#papo", "from this device", &[])
        .unwrap();
    session.handle_line(
        now,
        b"@label=0;msgid=RfcBTzyxY120004 :papo!p@localhost PRIVMSG #papo :from this device",
    );
    session.handle_line(
        now,
        b"@msgid=RfcBTzyxY120005 :papo!p@localhost PRIVMSG #papo :from another device",
    );

    let messages: Vec<_> = std::iter::from_fn(|| session.poll_event())
        .filter_map(|event| match event {
            Event::Message(message) => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(messages.len(), 2);
    assert!(messages[0].is_echo());
    assert!(!messages[0].is_other_session());
    assert!(!messages[1].is_echo());
    assert!(messages[1].is_other_session());
}

#[test]
fn labelled_response_batch_also_emits_its_message_echo() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo").wanted_caps([
        "batch",
        "echo-message",
        "labeled-response",
    ]));
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :batch echo-message labeled-response");
    session.handle_line(now, b"CAP * ACK :batch echo-message labeled-response");
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}
    session
        .send_text(now, TextKind::Privmsg, "#papo", "from this device", &[])
        .unwrap();
    session.handle_line(now, b"@label=0 BATCH +response labeled-response");
    session.handle_line(
        now,
        b"@batch=response;msgid=RfcBTzyxY120004 :papo!p@localhost PRIVMSG #papo :from this device",
    );
    session.handle_line(now, b"BATCH -response");

    let echo = std::iter::from_fn(|| session.poll_event()).find_map(|event| match event {
        Event::Message(message) => Some(message),
        _ => None,
    });
    assert!(echo.is_some_and(|message| message.is_echo() && !message.is_other_session()));
}

#[test]
fn requests_use_labels_only_when_negotiated() {
    let now = Instant::now();
    let mut labelled = Session::new(Config::default());
    labelled.handle_connected(now);
    labelled.handle_line(now, b"CAP * LS :labeled-response");
    labelled.handle_line(now, b"CAP * ACK :labeled-response");
    while labelled.poll_transmit().is_some() {}
    let first = labelled.request(now, Message::new("WHOIS", ["alice"]));
    let second = labelled.request(now, Message::privmsg("#papo", "hello"));
    assert_ne!(first, second);
    assert_eq!(
        labelled.poll_transmit(),
        Some(b"@label=0 WHOIS alice\r\n".to_vec())
    );
    assert_eq!(
        labelled.poll_transmit(),
        Some(b"@label=1 PRIVMSG #papo hello\r\n".to_vec())
    );

    let mut unlabelled = Session::new(Config::default());
    let id = unlabelled
        .request(now, Message::new("MODE", ["#papo"]))
        .unwrap();
    assert_eq!(unlabelled.poll_transmit(), Some(b"MODE #papo\r\n".to_vec()));
    assert!(matches!(
        unlabelled.poll_event(),
        Some(Event::RequestUntracked { id: event_id }) if event_id == id
    ));
    assert_eq!(
        unlabelled.request(now, Message::new("", ["broken"])),
        Err(RequestError::Build(BuildError::InvalidCommand))
    );
}

#[test]
fn request_errors_describe_the_failure_and_preserve_build_sources() {
    let build = RequestError::Build(BuildError::InvalidCommand);
    assert_eq!(build.to_string(), "IRC command is invalid");
    assert_eq!(
        std::error::Error::source(&build).and_then(|source| source.downcast_ref::<BuildError>()),
        Some(&BuildError::InvalidCommand)
    );

    let limit = RequestError::PendingLimit;
    assert_eq!(limit.to_string(), "too many labelled requests are pending");
    assert!(std::error::Error::source(&limit).is_none());
}
