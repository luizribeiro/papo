//! Transcript tests for session-owned ISUPPORT state.

mod support;

use papo_core::{Config, Event};
use papo_proto::Advertised;
use support::Transcript;

#[test]
fn applies_multiple_005_lines_and_enforces_utf8only() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test 005 papo CASEMAPPING=ascii NICKLEN=32 :are supported by this server")
        .server(":ergo.test 005 papo NETWORK=Ergo UTF8ONLY :are supported by this server")
        .inspect(|session| {
            assert_eq!(session.isupport().nicklen(), Some(32));
            assert_eq!(session.isupport().network(), Some("Ergo"));
            assert_eq!(session.isupport().get("UTF8ONLY"), Advertised::Bare);
        })
        .server_bytes(b":ergo.test NOTICE papo :caf\xe9")
        .event(|event| {
            event
                == &Event::Malformed {
                    line: b":ergo.test NOTICE papo :caf\xe9".to_vec(),
                }
        });
}

#[test]
fn removing_utf8only_restores_the_configured_decoding_policy() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":soju.example 005 papo UTF8ONLY :are supported by this server")
        .server(":soju.example 005 papo -UTF8ONLY :are supported by this server")
        .server_bytes(b":soju.example NOTICE papo :caf\xe9")
        .event(|event| matches!(event, Event::Message(_)));
}
