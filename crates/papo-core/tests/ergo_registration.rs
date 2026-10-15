//! Replay coverage for a registration exchange recorded from Ergo.

mod support;

use papo_core::{Config, Event};
use support::Transcript;

#[test]
fn replays_recorded_ergo_registration() {
    let config = Config::default()
        .nick("papo-fixture")
        .username("papo")
        .realname("papo registration fixture");
    let mut transcript = Transcript::new(config)
        .connect()
        .load("tests/data/ergo-registration.irc");
    for _ in 0..10 {
        transcript = transcript.event(|event| matches!(event, Event::Message(_)));
    }
    transcript.event(|event| {
        matches!(
            event,
            Event::Registered(welcome)
                if welcome.nick == "papo-fixture"
                    && welcome.caps.contains_key("server-time")
                    && welcome.isupport.network() == Some("ErgoTest")
                    && welcome.isupport.nicklen() == Some(32)
                    && welcome.isupport.utf8only()
        )
    });
}
