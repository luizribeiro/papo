//! Tests for the public session queue surface.

mod support;

use std::time::{Duration, Instant};

use papo_core::sts::ConnectionSecurity;
use papo_core::{
    Authorship, Config, DEFAULT_WANTED_CAPS, Event, Feature, RateLimit, RequestError, Secret,
    Session, TextKind, UnsupportedReason,
};
use papo_proto::{BuildError, Line};
use support::unsupported_error;

#[test]
fn config_builders_set_registration_identity_and_timeout() {
    let config = Config::default()
        .nick("emily")
        .username("em")
        .realname("Emily Example")
        .password("correct horse battery staple")
        .welcome_timeout(Duration::from_secs(23))
        .ping_interval(Duration::from_secs(41))
        .ping_timeout(Duration::from_secs(82))
        .rate_limit(RateLimit::new(7, Duration::from_millis(250)).unwrap())
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
        config.password,
        Some(Secret::new("correct horse battery staple"))
    );
    assert_eq!(config.connection_security, ConnectionSecurity::Secure);
    assert_eq!(config.host, "irc.example.net");
    assert_eq!(config.welcome_timeout, Duration::from_secs(23));
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
        RateLimit::new(7, Duration::from_millis(250)).unwrap()
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
    session.send(now, Line::new("NICK", ["tester"])).unwrap();
    session.send(now, Line::join("#papo")).unwrap();

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
    session.send(now, Line::new("NICK", ["tester"])).unwrap();
    session.handle_disconnected(now);
    assert_eq!(session.poll_transmit(), None);
}

#[test]
fn rejects_an_invalid_message_before_queueing_a_valid_one() {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    assert_eq!(
        session.send(now, Line::new("", ["tester"])),
        Err(BuildError::InvalidCommand)
    );
    assert_eq!(session.send(now, Line::join("#papo")), Ok(()));
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
fn annotates_authorship_with_the_active_casemapping() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("Papo[bot]"));
    session.handle_line(now, b":papo{bot}!~papo@localhost PRIVMSG #papo :sent");
    session.handle_line(now, b":alice!~a@localhost PRIVMSG #papo :received");

    let Some(Event::Line(echo)) = session.poll_event() else {
        panic!("expected echoed message")
    };
    let Some(Event::Line(received)) = session.poll_event() else {
        panic!("expected received message")
    };
    assert_eq!(echo.authorship(), Authorship::Own);
    assert_eq!(received.authorship(), Authorship::Other);
}

#[test]
fn labelled_echo_is_consumed_before_an_own_live_line() {
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
            Event::Line(message) => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].as_ref().param(1), Some("from another device"));
    assert_eq!(messages[0].authorship(), Authorship::Own);
}

#[test]
fn labelled_response_batch_does_not_emit_its_message_echo() {
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

    let events = std::iter::from_fn(|| session.poll_event()).collect::<Vec<_>>();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Response(_)))
    );
    assert!(!events.iter().any(|event| matches!(event, Event::Line(_))));
}

fn pending_labelled_echo(now: Instant) -> Session {
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
        .send_text(now, TextKind::Privmsg, "#papo", "hello from papo", &[])
        .unwrap();
    session
}

#[test]
fn labelled_echo_batch_does_not_turn_our_tagmsg_into_a_message() {
    let now = Instant::now();
    let mut session = pending_labelled_echo(now);
    session.handle_line(now, b"@label=0 BATCH +response labeled-response");
    session.handle_line(
        now,
        b"@batch=response;+typing=active :papo!p@localhost TAGMSG #papo",
    );
    session.handle_line(now, b"BATCH -response");

    assert!(
        std::iter::from_fn(|| session.poll_event()).all(|event| !matches!(event, Event::Line(_)))
    );
}

#[test]
fn labelled_echo_batch_does_not_dispatch_another_users_message() {
    let now = Instant::now();
    let mut session = pending_labelled_echo(now);
    session.handle_line(now, b"@label=0 BATCH +response labeled-response");
    session.handle_line(
        now,
        b"@batch=response;msgid=RfcBTzyxY120005 :alice!a@localhost PRIVMSG #papo :hello from Alice",
    );
    session.handle_line(now, b"BATCH -response");

    assert!(
        std::iter::from_fn(|| session.poll_event()).all(|event| !matches!(event, Event::Line(_)))
    );
}

#[test]
fn raw_requests_require_negotiated_labels() {
    let now = Instant::now();
    let mut labelled = Session::new(Config::default());
    labelled.handle_connected(now);
    labelled.handle_line(now, b"CAP * LS :labeled-response");
    labelled.handle_line(now, b"CAP * ACK :labeled-response");
    while labelled.poll_transmit().is_some() {}
    let first = labelled.request(now, Line::new("WHOIS", ["alice"]));
    let second = labelled.request(now, Line::privmsg("#papo", "hello"));
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
    assert_eq!(
        unlabelled.request(now, Line::new("MODE", ["#papo"])),
        Err(unsupported_error(Feature::LabeledResponse))
    );
    assert_eq!(unlabelled.poll_transmit(), None);
    assert_eq!(
        labelled.request(now, Line::new("", ["broken"])),
        Err(RequestError::Build(BuildError::InvalidCommand))
    );
}

#[test]
fn raw_requests_preflight_features_named_by_the_message() {
    let now = Instant::now();
    let mut session =
        Session::new(Config::default().wanted_caps(["labeled-response", "message-tags"]));
    session.handle_connected(now);
    session.handle_line(
        now,
        b"CAP * LS :labeled-response message-tags draft/message-redaction",
    );
    session.handle_line(now, b"CAP * ACK :labeled-response message-tags");
    session.handle_line(
        now,
        b":irc.solanum.example 005 papo CLIENTTAGDENY=draft/react,typing :are supported",
    );
    while session.poll_transmit().is_some() {}

    for (message, feature, reason) in [
        (
            Line::new("REDACT", ["#papo", "RfcBTzyxY120005"]),
            Feature::Redaction,
            UnsupportedReason::NotOffered,
        ),
        (
            Line::tagmsg("#papo").with_tag("+draft/react", "👍"),
            Feature::Reactions,
            UnsupportedReason::TagDenied,
        ),
        (
            Line::tagmsg("#papo").with_tag("+typing", "active"),
            Feature::Typing,
            UnsupportedReason::TagDenied,
        ),
    ] {
        assert_eq!(
            session.request(now, message),
            Err(support::unsupported_because(feature, reason))
        );
    }
    assert_eq!(session.poll_transmit(), None);
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
    assert_eq!(
        unsupported_error(Feature::History).to_string(),
        "chathistory unavailable: the server did not offer it"
    );
    assert_eq!(
        RequestError::InvalidMessageReference.to_string(),
        "unsupported message reference"
    );
}
