//! Transcript tests for session-owned ISUPPORT state.

mod support;

use papo_core::{Config, Event, Session, StateLimit};
use papo_proto::Advertised;
use support::Transcript;

#[test]
fn applies_multiple_005_lines_and_enforces_utf8only() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test 005 papo CASEMAPPING=ascii NICKLEN=32 :are supported by this server")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .server(":ergo.test 005 papo NETWORK=Ergo UTF8ONLY :are supported by this server")
        .inspect(|session| {
            assert_eq!(session.isupport().nicklen(), Some(32));
            assert_eq!(session.isupport().network(), Some("Ergo"));
            assert_eq!(session.isupport().get("UTF8ONLY"), Advertised::Bare);
        })
        .server_bytes(b":ergo.test NOTICE papo :caf\xe9")
        .event(|event| event == &Event::__malformed(b":ergo.test NOTICE papo :caf\xe9".to_vec()));
}

#[test]
fn removing_utf8only_restores_the_configured_decoding_policy() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":soju.example 005 papo UTF8ONLY :are supported by this server")
        .server(":soju.example 005 papo -UTF8ONLY :are supported by this server")
        .server_bytes(b":soju.example NOTICE papo :caf\xe9")
        .event(|event| matches!(event, Event::Line(_)));
}

#[test]
fn warns_once_per_connection_when_rfc8265_cannot_be_applied() {
    let now = std::time::Instant::now();
    let mut session = Session::new(Config::default());
    for _ in 0..2 {
        session.handle_connected(now);
        while session.poll_event().is_some() {}
        for line in [
            ":ergo.test 005 papo CASEMAPPING=ascii UTF8MAPPING=rfc8265 :are supported",
            ":ergo.test 005 papo UTF8MAPPING=rfc8265 :are supported",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        let warnings = std::iter::from_fn(|| session.poll_event())
            .filter(|event| matches!(event, Event::Utf8CasemappingUnsupported))
            .count();
        let supports_rfc8265 = session.isupport().casemapping().as_str() == "rfc7613";
        assert_eq!(warnings, usize::from(!supports_rfc8265));
        session.handle_disconnected(now);
    }
}

#[test]
fn warns_once_when_isupport_storage_reaches_its_bound() {
    Transcript::new(Config::default())
        .connect_default()
        .update(|session| {
            let now = std::time::Instant::now();
            for index in 0..5_000 {
                session.handle_line(
                    now,
                    format!(":ergo.test 005 papo VENDOR{index}=present :are supported").as_bytes(),
                );
            }
            session.handle_line(
                now,
                b":ergo.test 005 papo CASEMAPPING=ascii PREFIX=(qaohv)~&@%+ CHANTYPES=# :are supported",
            );
        })
        .event(|event| {
            matches!(
                event,
                Event::StateLimitReached {
                    limit: StateLimit::IsupportTokens,
                    ..
                }
            )
        })
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .inspect(|session| {
            assert_eq!(session.isupport().tokens().len(), 1_024);
            assert_eq!(session.isupport().chantypes(), "#");
            assert_eq!(session.isupport().prefix().len(), 5);
        })
        .no_event();
}
