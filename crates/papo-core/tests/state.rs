//! Transcript tests for channel and user state.

mod support;

use papo_core::{Config, Event, StateLimit};
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

#[test]
fn loads_a_split_ergo_names_burst_with_ranked_userhosts() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily PREFIX=(qaohv)~&@%+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":ergo.test 353 emily = #papo :@+emily!em@127.0.0.1 dan!~dan@cloak.example")
        .server(":ergo.test 353 emily = #papo :+alice!alice@staff.example")
        .server(":ergo.test 366 emily #papo :End of NAMES list")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert!(channel.is_synced());
            assert_eq!(channel.members().count(), 3);
            let emily = channel.member("emily").unwrap();
            assert_eq!(emily.prefixes(), ['@', '+']);
            assert!(emily.is_op());
            assert!(!channel.member("alice").unwrap().is_op());
            assert_eq!(
                session.state().user("dan").unwrap().host(),
                Some("cloak.example")
            );
        });
}

#[test]
fn no_implicit_names_marks_our_join_synced_without_366() {
    Transcript::new(Config::default().wanted_caps(["no-implicit-names"]))
        .connect_default()
        .server("CAP * LS :draft/no-implicit-names")
        .client("CAP REQ :draft/no-implicit-names")
        .server("CAP * ACK :draft/no-implicit-names")
        .client("CAP END")
        .server(":papo!papo@127.0.0.1 JOIN #papo")
        .inspect(|session| assert!(session.state().channel("#papo").unwrap().is_synced()));
}

#[test]
fn removes_other_members_on_part_kick_and_multi_channel_quit() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":emily!em@127.0.0.1 JOIN #rust")
        .server(":dan!dan@cloak.example JOIN #papo")
        .server(":dan!dan@cloak.example JOIN #rust")
        .server(":alice!alice@staff.example JOIN #papo")
        .server(":bob!bob@users.example JOIN #papo")
        .server(":alice!alice@staff.example PART #papo :Leaving")
        .server(":chanop!op@staff.example KICK #papo bob :Rule violation")
        .inspect(|session| {
            assert!(session.state().user("alice").is_none());
            assert!(session.state().user("bob").is_none());
            assert_eq!(session.state().user("dan").unwrap().channels().count(), 2);
        })
        .server(":dan!dan@cloak.example QUIT :Client exited")
        .inspect(|session| {
            assert!(session.state().user("dan").is_none());
            assert!(
                session
                    .state()
                    .channel("#papo")
                    .unwrap()
                    .member("dan")
                    .is_none()
            );
            assert!(
                session
                    .state()
                    .channel("#rust")
                    .unwrap()
                    .member("dan")
                    .is_none()
            );
        });
}

#[test]
fn our_part_and_kick_remove_channels_and_orphaned_users() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":dan!dan@cloak.example JOIN #papo")
        .server(":emily!em@127.0.0.1 PART #papo :Leaving")
        .server(":emily!em@127.0.0.1 JOIN #rust")
        .server(":chanop!op@staff.example KICK #rust emily :Requested")
        .inspect(|session| {
            assert_eq!(session.state().channels().count(), 0);
            assert!(session.state().user("dan").is_none());
        });
}

#[test]
fn rekeys_another_users_nick_across_shared_channels() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":emily!em@127.0.0.1 JOIN #rust")
        .server(":dan!dan@cloak.example JOIN #papo")
        .server(":dan!dan@cloak.example JOIN #rust")
        .server(":dan!dan@cloak.example NICK :dan-away")
        .inspect(|session| {
            assert!(session.state().user("dan").is_none());
            assert_eq!(session.state().user("DAN-AWAY").unwrap().nick(), "dan-away");
            for channel in session.state().channels() {
                assert!(channel.member("dan").is_none());
                assert!(channel.member("dan-away").is_some());
            }
        });
}

#[test]
fn follows_our_own_nick_change() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":emily!em@127.0.0.1 NICK :emily_")
        .inspect(|session| {
            assert_eq!(session.state().nick(), "emily_");
            let channel = session.state().channel("#papo").unwrap();
            assert!(channel.member("emily").is_none());
            assert!(channel.member("EMILY_").is_some());
        });
}

#[test]
fn bounds_channels_and_members_with_one_warning_each() {
    Transcript::new(
        Config::default()
            .nick("emily")
            .max_channels(1)
            .max_members_per_channel(2),
    )
    .server(":emily!em@127.0.0.1 JOIN #papo")
    .server(":emily!em@127.0.0.1 JOIN #rust")
    .event(|event| {
        matches!(
            event,
            Event::StateLimitReached {
                limit: StateLimit::Channels
            }
        )
    })
    .server(":emily!em@127.0.0.1 JOIN #bots")
    .server(":dan!dan@cloak.example JOIN #papo")
    .server(":alice!alice@staff.example JOIN #papo")
    .event(|event| {
        matches!(
            event,
            Event::StateLimitReached {
                limit: StateLimit::Members
            }
        )
    })
    .server(":bob!bob@users.example JOIN #papo")
    .inspect(|session| {
        assert_eq!(session.state().channels().count(), 1);
        let channel = session.state().channel("#papo").unwrap();
        assert_eq!(channel.members().count(), 2);
        assert!(channel.member("alice").is_none());
        assert!(channel.member("bob").is_none());
    });
}
