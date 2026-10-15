//! Transcript tests for channel and user state.

mod support;

use papo_core::Config;
use support::Transcript;

#[test]
fn starts_with_configured_identity_and_empty_shared_state() {
    Transcript::new(Config::default().nick("emily").username("em"))
        .inspect(|session| {
            let state = session.state();
            assert_eq!(state.nick(), "emily");
            assert_eq!(state.username(), "em");
            assert_eq!(state.host(), None);
        })
        .no_event();
}

#[test]
fn tracks_our_join_and_another_member_with_rfc1459_keys() {
    Transcript::new(Config::default().nick("emily").username("em"))
        .server(":emily!em@127.0.0.1 JOIN #Papo[dev]")
        .server(":dan!dan@127.0.0.2 JOIN #papo{dev}")
        .inspect(|session| {
            let channel = session.state().channel("#PAPO{DEV}").unwrap();
            assert_eq!(channel.member("EMILY").unwrap().nick(), "emily");
            assert_eq!(channel.member("dan").unwrap().nick(), "dan");
        });
}

#[test]
fn tracks_userhost_and_shared_channels_from_joins() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":dan!~dan@cloak.example JOIN #papo")
        .server(":emily!em@127.0.0.1 JOIN #rust")
        .server(":dan!~dan@cloak.example JOIN #rust")
        .inspect(|session| {
            let dan = session.state().user("DAN").unwrap();
            assert_eq!(dan.nick(), "dan");
            assert_eq!(dan.username(), Some("~dan"));
            assert_eq!(dan.host(), Some("cloak.example"));
            assert_eq!(dan.channels().count(), 2);
        });
}
