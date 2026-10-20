//! Replay coverage for a registration exchange recorded from Ergo.

mod support;

use papo_core::{Config, Event};
use support::Transcript;

#[test]
fn replays_recorded_ergo_registration() {
    let config = Config::default()
        .nick("papo-fixture")
        .username("papo")
        .realname("papo registration fixture")
        .without_caps(["draft/persistence", "draft/relaymsg"]);
    Transcript::new(config)
        .connect()
        .load("tests/data/ergo-registration.irc")
        .event_until(|event| match event {
            Event::Line(_) => false,
            Event::Registered(welcome) => {
                assert_eq!(welcome.nick, "papo-fixture");
                assert!(welcome.caps.contains_key("server-time"));
                assert_eq!(welcome.isupport.network(), Some("ErgoTest"));
                assert_eq!(welcome.isupport.nicklen(), Some(32));
                assert!(welcome.isupport.utf8only());
                true
            }
            other => panic!("unexpected event during registration: {other:?}"),
        });
}
