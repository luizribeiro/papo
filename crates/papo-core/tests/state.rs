//! Transcript tests for channel and user state.

mod support;

use std::time::{Duration, UNIX_EPOCH};

use papo_core::{Account, Config, Event, OwnedModeChange, RateLimit, StateLimit};
use papo_proto::{Message, ModeKind};
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
fn welcome_adopts_the_server_assigned_nick_in_state() {
    Transcript::new(Config::default().nick("oldnick"))
        .welcome(":irc.example 001 account :Welcome to the network")
        .inspect(|session| assert_eq!(session.state().nick(), "account"))
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
        })
        .ignore_rest();
}

#[test]
fn exposes_our_host_and_original_channel_metadata() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #Papo")
        .event(|event| matches!(event, Event::Joined { channel, nick } if channel == "#Papo" && nick == "emily"))
        .inspect(|session| {
            assert_eq!(session.state().host(), Some("127.0.0.1"));
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.name(), "#Papo");
            assert_eq!(channel.topic(), None);
        })
        .no_event();
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
        })
        .ignore_rest();
}

#[test]
fn extended_join_distinguishes_accounts_from_logged_out_users() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo emily :Emily")
        .server(":dan!~dan@cloak.example JOIN #papo dan :Daniel Garcia")
        .server(":alice!alice@staff.example JOIN #papo * :Alice Smith")
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().account(),
                &Account::LoggedIn("dan".into())
            );
            assert_eq!(
                session.state().user("alice").unwrap().account(),
                &Account::LoggedOut
            );
            assert_eq!(
                session.state().user("dan").unwrap().realname(),
                Some("Daniel Garcia")
            );
        })
        .ignore_rest();
}

#[test]
fn account_notify_tracks_login_and_logout() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":dan!~dan@cloak.example JOIN #papo")
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().account(),
                &Account::Unknown
            );
        })
        .server(":dan!~dan@cloak.example ACCOUNT dan")
        .event_until(|event| matches!(event, Event::AccountChanged { nick, account } if nick == "dan" && account == &Account::LoggedIn("dan".into())))
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().account(),
                &Account::LoggedIn("dan".into())
            );
        })
        .server(":dan!~dan@cloak.example ACCOUNT *")
        .event(|event| matches!(event, Event::AccountChanged { nick, account } if nick == "dan" && account == &Account::LoggedOut))
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().account(),
                &Account::LoggedOut
            );
        })
        .ignore_rest();
}

#[test]
fn account_tag_fills_an_unknown_account() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":dan!~dan@cloak.example JOIN #papo")
        .server("@account=dan :dan!~dan@cloak.example PRIVMSG #papo :hello Emily")
        .event_until(|event| matches!(event, Event::AccountChanged { nick, account } if nick == "dan" && account == &Account::LoggedIn("dan".into())))
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().account(),
                &Account::LoggedIn("dan".into())
            );
        })
        .ignore_rest();
}

#[test]
fn away_notify_sets_and_clears_a_joined_users_message() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":dan!~dan@cloak.example JOIN #papo dan :Daniel Garcia")
        .server(":dan!~dan@cloak.example AWAY :Gone to lunch")
        .event_until(|event| matches!(event, Event::AwayChanged { nick, message } if nick == "dan" && message.as_deref() == Some("Gone to lunch")))
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().away(),
                Some("Gone to lunch")
            );
        })
        .server(":dan!~dan@cloak.example AWAY")
        .event(|event| matches!(event, Event::AwayChanged { nick, message } if nick == "dan" && message.is_none()))
        .inspect(|session| assert_eq!(session.state().user("dan").unwrap().away(), None))
        .ignore_rest();
}

#[test]
fn away_notify_immediately_after_join_records_initial_state() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":alice!alice@staff.example JOIN #papo alice :Alice Smith")
        .server(":alice!alice@staff.example AWAY :Working")
        .inspect(|session| {
            assert_eq!(
                session.state().user("alice").unwrap().away(),
                Some("Working")
            );
        })
        .ignore_rest();
}

#[test]
fn whois_away_reply_records_the_message() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":dan!~dan@cloak.example JOIN #papo")
        .server(":ergo.test 301 emily dan :Gone to lunch")
        .event_until(|event| matches!(event, Event::AwayChanged { nick, message } if nick == "dan" && message.as_deref() == Some("Gone to lunch")))
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().away(),
                Some("Gone to lunch")
            );
        })
        .ignore_rest();
}

#[test]
fn away_numerics_track_our_own_state() {
    Transcript::new(Config::default().nick("emily"))
        .inspect(|session| assert!(!session.state().away()))
        .server(":ergo.test 306 emily :You have been marked as being away")
        .event(|event| matches!(event, Event::AwayChanged { nick, message } if nick == "emily" && message.as_deref() == Some("")))
        .inspect(|session| assert!(session.state().away()))
        .server(":ergo.test 305 emily :You are no longer marked as being away")
        .event(|event| matches!(event, Event::AwayChanged { nick, message } if nick == "emily" && message.is_none()))
        .inspect(|session| assert!(!session.state().away()))
        .no_event();
}

#[test]
fn away_numerics_do_not_repeat_our_own_state_changes() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 306 emily :You have been marked as being away")
        .event(|event| {
            matches!(
                event,
                Event::AwayChanged {
                    message: Some(_),
                    ..
                }
            )
        })
        .server(":ergo.test 306 emily :You have been marked as being away")
        .no_event()
        .inspect(|session| assert!(session.state().away()))
        .server(":ergo.test 305 emily :You are no longer marked as being away")
        .event(|event| matches!(event, Event::AwayChanged { message: None, .. }))
        .server(":ergo.test 305 emily :You are no longer marked as being away")
        .no_event()
        .inspect(|session| assert!(!session.state().away()))
        .no_event();
}

#[test]
fn chghost_updates_shared_and_own_userhosts() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":dan!~dan@cloak.example JOIN #papo")
        .server(":dan!~dan@cloak.example CHGHOST dan account/dan")
        .event_until(|event| matches!(event, Event::HostChanged { nick, user, host } if nick == "dan" && user == "dan" && host == "account/dan"))
        .server(":emily!em@127.0.0.1 CHGHOST ~em users/emily")
        .event(|event| matches!(event, Event::HostChanged { nick, user, host } if nick == "emily" && user == "~em" && host == "users/emily"))
        .inspect(|session| {
            let dan = session.state().user("dan").unwrap();
            assert_eq!(dan.username(), Some("dan"));
            assert_eq!(dan.host(), Some("account/dan"));
            assert_eq!(session.state().username(), "~em");
            assert_eq!(session.state().host(), Some("users/emily"));
        })
        .ignore_rest();
}

#[test]
fn visible_host_numeric_updates_our_host() {
    Transcript::new(Config::default().nick("emily").username("~em"))
        .server(":ergo.test 396 emily gateway/tor-sasl :is now your displayed host")
        .event(|event| matches!(event, Event::HostChanged { nick, user, host } if nick == "emily" && user == "~em" && host == "gateway/tor-sasl"))
        .inspect(|session| assert_eq!(session.state().host(), Some("gateway/tor-sasl")))
        .no_event();
}

#[test]
fn setname_updates_other_and_own_realnames() {
    Transcript::new(Config::default().nick("emily").realname("Emily Garcia"))
        .server(":emily!em@127.0.0.1 JOIN #papo emily :Emily Garcia")
    .server(":dan!~dan@cloak.example JOIN #papo dan :Daniel Garcia")
    .server(":dan!~dan@cloak.example SETNAME :Dan Garcia")
    .event_until(|event| matches!(event, Event::RealnameChanged { nick, realname } if nick == "dan" && realname == "Dan Garcia"))
    .server(":emily!em@127.0.0.1 SETNAME :Emily Smith")
    .event(|event| matches!(event, Event::RealnameChanged { nick, realname } if nick == "emily" && realname == "Emily Smith"))
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().realname(),
                Some("Dan Garcia")
            );
            assert_eq!(session.state().realname(), "Emily Smith");
            assert_eq!(
                session.state().user("emily").unwrap().realname(),
                Some("Emily Smith")
            );
        })
        .ignore_rest();
}

#[test]
fn a_later_join_fills_a_bare_names_userhost() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "emily"))
        .server(":ergo.test 353 emily = #papo :dan")
        .inspect(|session| {
            let dan = session.state().user("dan").unwrap();
            assert_eq!(dan.username(), None);
            assert_eq!(dan.host(), None);
        })
        .server(":dan!~dan@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { channel, nick } if channel == "#papo" && nick == "dan"))
        .inspect(|session| {
            let dan = session.state().user("dan").unwrap();
            assert_eq!(dan.username(), Some("~dan"));
            assert_eq!(dan.host(), Some("cloak.example"));
        })
        .no_event();
}

#[test]
fn loads_a_split_ergo_names_burst_with_ranked_userhosts() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily PREFIX=(qaohv)~&@%+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":ergo.test 353 emily = #papo :@+emily!em@127.0.0.1 dan!~dan@cloak.example")
        .server(":ergo.test 353 emily = #papo :+alice!alice@staff.example")
        .server(":ergo.test 366 emily #papo :End of NAMES list")
        .event_until(
            |event| matches!(event, Event::ChannelSynced { channel } if channel == "#papo"),
        )
        .client("MODE #papo")
        .client("WHO #papo")
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
        })
        .no_event();
}

#[test]
fn ranks_names_prefixes_independently_of_wire_order() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily PREFIX=(qaohv)~&@%+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "emily"))
        .server(":ergo.test 353 emily = #papo :+@emily!em@127.0.0.1")
        .inspect(|session| {
            assert_eq!(
                session
                    .state()
                    .channel("#papo")
                    .unwrap()
                    .member("emily")
                    .unwrap()
                    .prefixes(),
                ['@', '+']
            );
        })
        .no_event();
}

#[test]
fn applies_membership_modes_in_advertised_rank_order() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily CHANMODES=Ibe,k,fl,CEMRUimnstu PREFIX=(qaohv)~&@%+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":ergo.test 353 emily = #papo :+emily!em@127.0.0.1 dan!dan@cloak.example")
        .server(":alice!alice@staff.example MODE #papo +ov dan emily")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.member("dan").unwrap().prefixes(), ['@']);
            assert_eq!(channel.member("emily").unwrap().prefixes(), ['+']);
        })
        .server(":alice!alice@staff.example MODE #papo +v-o dan dan")
        .inspect(|session| {
            assert_eq!(
                session
                    .state()
                    .channel("#papo")
                    .unwrap()
                    .member("dan")
                    .unwrap()
                    .prefixes(),
                ['+']
            );
        })
        .ignore_rest();
}

#[test]
fn ignores_membership_modes_for_unknown_channels_and_members() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily CHANMODES=Ibe,k,fl,CEMRUimnstu PREFIX=(ov)@+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":alice!alice@staff.example MODE #unknown +o emily")
        .server(":alice!alice@staff.example MODE #papo +o dan")
        .inspect(|session| {
            assert_eq!(
                session
                    .state()
                    .channel("#papo")
                    .unwrap()
                    .member("emily")
                    .unwrap()
                    .prefixes(),
                []
            );
        })
        .ignore_rest();
}

#[test]
fn tracks_the_channel_join_burst_and_live_mode_changes() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily CHANMODES=Ibe,k,fl,CEMRUimnstu PREFIX=(ov)@+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":ergo.test 324 emily #papo +kln swordfish 10")
        .server(":ergo.test 329 emily #papo 1723501539")
        .server(":ergo.test 332 emily #papo :Welcome to the papo channel")
        .server(":ergo.test 333 emily #papo alice 1723501612")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.mode('k'), Some(Some("swordfish")));
            assert_eq!(channel.mode('l'), Some(Some("10")));
            assert_eq!(channel.mode('n'), Some(None));
            assert_eq!(
                channel.created_at(),
                Some(UNIX_EPOCH + Duration::from_secs(1_723_501_539))
            );
            assert_eq!(channel.topic(), Some("Welcome to the papo channel"));
            assert_eq!(channel.topic_set_by(), Some("alice"));
            assert_eq!(
                channel.topic_set_at(),
                Some(UNIX_EPOCH + Duration::from_secs(1_723_501_612))
            );
        })
        .no_event()
        .server(":alice!alice@staff.example MODE #papo -k")
        .server(":alice!alice@staff.example MODE #papo -l+mb *!*@gateway/tor-sasl")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.mode('k'), None);
            assert_eq!(channel.mode('l'), None);
            assert_eq!(channel.mode('m'), Some(None));
            assert_eq!(channel.mode('b'), None);
        })
        .server(":ergo.test 324 emily #papo +n")
        .inspect(|session| assert_eq!(session.state().channel("#papo").unwrap().modes().count(), 1))
        .ignore_rest();
}

#[test]
fn tracks_live_topic_changes_and_clearing() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":alice!alice@staff.example TOPIC #papo :Rust 1.90 is out")
        .event(|event| matches!(event, Event::TopicChanged { channel, topic, by } if channel == "#papo" && topic.as_deref() == Some("Rust 1.90 is out") && by == "alice"))
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.topic(), Some("Rust 1.90 is out"));
            assert_eq!(channel.topic_set_by(), Some("alice"));
            assert_eq!(channel.topic_set_at(), None);
        })
        .server(":alice!alice@staff.example TOPIC #papo :")
        .event(|event| matches!(event, Event::TopicChanged { channel, topic, by } if channel == "#papo" && topic.is_none() && by == "alice"))
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.topic(), None);
            assert_eq!(channel.topic_set_by(), Some("alice"));
        })
        .no_event();
}

#[test]
fn reports_mixed_channel_and_user_mode_changes() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily CHANMODES=Ibe,k,fl,CEMRUimnstu PREFIX=(ov)@+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":alice!alice@staff.example MODE #papo +o-b emily *!*@gateway/tor-sasl")
        .event(|event| match event {
            Event::ModeChanged { target, by, changes } => {
                target == "#papo"
                    && by == "alice"
                    && changes == &vec![
                        OwnedModeChange { set: true, mode: 'o', arg: Some("emily".into()), kind: ModeKind::Prefix(0) },
                        OwnedModeChange { set: false, mode: 'b', arg: Some("*!*@gateway/tor-sasl".into()), kind: ModeKind::List },
                    ]
            }
            _ => false,
        })
        .server(":ergo.test MODE emily +i")
        .event(|event| matches!(event, Event::ModeChanged { target, by, changes } if target == "emily" && by == "ergo.test" && changes == &vec![OwnedModeChange { set: true, mode: 'i', arg: None, kind: ModeKind::Unknown }]))
        .no_event();
}

#[test]
fn tracks_user_mode_snapshot_and_live_changes() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 221 emily +Zi")
        .server(":emily MODE emily +w-i")
        .inspect(|session| {
            let mut modes = session.state().user_modes().collect::<Vec<_>>();
            modes.sort_unstable();
            assert_eq!(modes, ['Z', 'w']);
        })
        .server(":ergo.test 221 emily +i")
        .inspect(|session| assert_eq!(session.state().user_modes().collect::<Vec<_>>(), ['i']))
        .ignore_rest();
}

#[test]
fn only_our_nickname_changes_our_user_modes() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 221 emily +Z")
        .server(":ergo.test MODE dan +i")
        .inspect(|session| assert_eq!(session.state().user_modes().collect::<Vec<_>>(), ['Z']))
        .server(":ergo.test MODE emily +w")
        .inspect(|session| {
            let mut modes = session.state().user_modes().collect::<Vec<_>>();
            modes.sort_unstable();
            assert_eq!(modes, ['Z', 'w']);
        })
        .ignore_rest();
}

#[test]
fn distinguishes_prefix_rank_and_channel_mode_argument_branches() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily CHANMODES=Ibe,k,fl,CEMRUimnstu PREFIX=(qaohv)~&@%+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":ergo.test 353 emily = #papo :+dan!dan@cloak.example erin!erin@cloak.example @frank!frank@staff.example")
        .server(":alice!alice@staff.example MODE #papo +o dan")
        .server(":alice!alice@staff.example MODE #papo +l")
        .server(":alice!alice@staff.example MODE #papo +m")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.member("dan").unwrap().prefixes(), ['@', '+']);
            assert_eq!(channel.member("dan").unwrap().highest_prefix(), Some('@'));
            assert!(channel.member("dan").unwrap().is_voiced());
            assert!(channel.member("dan").unwrap().is_halfop());
            assert!(!channel.member("erin").unwrap().is_halfop());
            assert!(!channel.member("erin").unwrap().is_voiced());
            assert!(channel.member("frank").unwrap().is_op());
            assert!(!channel.member("frank").unwrap().is_voiced());
            assert_eq!(channel.mode('l'), None);
            assert_eq!(channel.mode('m'), Some(None));
        })
        .ignore_rest();
}

#[test]
fn bounds_channel_modes_at_literal_sixty_four() {
    let modes = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnpqrstuwxyzαβγδεζηθικλμνξο";
    let support = format!(":ergo.test 005 emily CHANMODES=,,,{modes} :are supported");
    let snapshot = format!(":ergo.test 324 emily #papo +{modes}");
    Transcript::new(Config::default().nick("emily"))
        .server(&support)
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(&snapshot)
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.modes().count(), 64);
            assert_eq!(channel.mode('ο'), None);
        })
        .ignore_rest();
}

#[test]
fn no_implicit_names_requests_names_before_marking_the_channel_synced() {
    Transcript::new(Config::default().wanted_caps(["no-implicit-names"]))
        .connect_default()
        .server("CAP * LS :draft/no-implicit-names")
        .client("CAP REQ :draft/no-implicit-names")
        .server("CAP * ACK :draft/no-implicit-names")
        .client("CAP END")
        .server(":ergo.test 005 papo WHOX :are supported")
        .server(":papo!papo@127.0.0.1 JOIN #papo")
        .client("NAMES #papo")
        .event(|event| matches!(event, Event::Joined { channel, nick } if channel == "#papo" && nick == "papo"))
        .inspect(|session| assert!(!session.state().channel("#papo").unwrap().is_synced()))
        .server(":ergo.test 353 papo = #papo :@papo +emily")
        .server(":ergo.test 366 papo #papo :End of NAMES list")
        .client("MODE #papo")
        .client("WHO #papo %tcuhnfar,152")
        .event(|event| matches!(event, Event::ChannelSynced { channel } if channel == "#papo"))
        .inspect(|session| assert!(session.state().channel("#papo").unwrap().is_synced()))
        .no_event();
}

#[test]
fn another_capability_waits_for_end_of_names_before_syncing() {
    Transcript::new(Config::default().wanted_caps(["multi-prefix"]))
        .connect_default()
        .server("CAP * LS :multi-prefix")
        .client("CAP REQ :multi-prefix")
        .server("CAP * ACK :multi-prefix")
        .client("CAP END")
        .server(":solanum.example 005 papo CHANMODES=Ibe,k,fl,imnpst :are supported")
        .server(":papo!papo@127.0.0.1 JOIN #Papo")
        .event(|event| matches!(event, Event::Joined { channel, nick } if channel == "#Papo" && nick == "papo"))
        .inspect(|session| assert!(!session.state().channel("#papo").unwrap().is_synced()))
        .server(":ergo.test 366 papo #Papo :End of NAMES list")
        .client("MODE #Papo")
        .client("WHO #Papo")
        .event(|event| matches!(event, Event::ChannelSynced { channel } if channel == "#Papo"))
        .server(":solanum.example 324 papo #Papo +ntk secret")
        .server(":solanum.example 329 papo #Papo 1760529342")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert!(channel.is_synced());
            assert_eq!(channel.mode('n'), Some(None));
            assert_eq!(channel.mode('k'), Some(Some("secret")));
            assert_eq!(channel.created_at(), Some(UNIX_EPOCH + Duration::from_secs(1_760_529_342)));
        })
        .no_event();
}

#[test]
fn exposes_channel_replies_for_unjoined_channels_as_messages() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 353 emily = #opers :@alice +dan")
        .event(
            |event| matches!(event, Event::Message(message) if message.as_ref().command_is("353")),
        )
        .server(":ergo.test 366 emily #opers :End of NAMES list")
        .event(
            |event| matches!(event, Event::Message(message) if message.as_ref().command_is("366")),
        )
        .server(":solanum.example 332 emily #staff :Operator coordination")
        .event(
            |event| matches!(event, Event::Message(message) if message.as_ref().command_is("332")),
        )
        .server(":solanum.example 333 emily #staff alice 1760529342")
        .event(
            |event| matches!(event, Event::Message(message) if message.as_ref().command_is("333")),
        )
        .server(":irc.unreal.test 324 emily #services +snt")
        .event(
            |event| matches!(event, Event::Message(message) if message.as_ref().command_is("324")),
        )
        .server(":irc.unreal.test 329 emily #services 1760529342")
        .event(
            |event| matches!(event, Event::Message(message) if message.as_ref().command_is("329")),
        )
        .inspect(|session| assert!(session.state().channel("#opers").is_none()))
        .no_event();
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
        })
        .ignore_rest();
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
        })
        .ignore_rest();
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
        })
        .ignore_rest();
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
        })
        .ignore_rest();
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
    .event_until(|event| {
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
    .event_until(|event| {
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
    })
    .ignore_rest();
}

#[test]
fn bot_tag_marks_a_shared_user() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":papo-bot!bot@services.example JOIN #papo")
        .server("@bot :papo-bot!bot@services.example PRIVMSG #papo :status green")
        .inspect(|session| assert!(session.state().user("papo-bot").unwrap().is_bot()))
        .ignore_rest();
}

#[test]
fn sweeps_joined_channels_with_a_rate_limited_whox_query() {
    Transcript::rate_limited(
        Config::default()
            .nick("emily")
            .rate_limit(RateLimit::new(1, Duration::from_millis(500))),
    )
    .connect()
    .client("CAP LS 302")
    .client("NICK emily")
    .client("USER papo 0 * papo")
    .send(Message::privmsg("#papo", "warmup"))
    .client("PRIVMSG #papo warmup")
    .server(":ergo.test 005 emily WHOX BOT=B PREFIX=(ov)@+ :are supported")
    .server(":emily!em@127.0.0.1 JOIN #papo")
    .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "emily"))
    .server(":ergo.test 353 emily = #papo :@emily!em@127.0.0.1 +dan!~dan@cloak.example")
    .server(":ergo.test 366 emily #papo :End of NAMES list")
    .no_client()
    .advance(Duration::from_millis(500))
    .client("MODE #papo")
    .no_client()
    .advance(Duration::from_millis(500))
    .client("WHO #papo %tcuhnfar,152")
    .event(|event| matches!(event, Event::ChannelSynced { channel } if channel == "#papo"))
    .server(":ergo.test 354 emily 152 #papo em 127.0.0.1 emily H@ emily :Emily Garcia")
    .server(":ergo.test 354 emily 152 #papo ~dan account/dan dan GB+ dan :Daniel Garcia")
    .server(":ergo.test 315 emily #papo :End of WHO list")
    .inspect(|session| {
        let dan = session.state().user("dan").unwrap();
        assert_eq!(dan.account(), &Account::LoggedIn("dan".into()));
        assert_eq!(dan.username(), Some("~dan"));
        assert_eq!(dan.host(), Some("account/dan"));
        assert_eq!(dan.realname(), Some("Daniel Garcia"));
        assert_eq!(dan.away(), Some(""));
        assert!(dan.is_bot());
    })
    .no_event();
}

#[test]
fn falls_back_to_plain_who_without_whox() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily BOT=B PREFIX=(ov)@+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "emily"))
        .server(":ergo.test 353 emily = #papo :emily!em@127.0.0.1 +dan!~dan@cloak.example")
        .server(":ergo.test 366 emily #papo :End of NAMES list")
        .client("MODE #papo")
        .client("WHO #papo")
        .event(|event| matches!(event, Event::ChannelSynced { channel } if channel == "#papo"))
        .server(":ergo.test 352 emily #papo ~dan account/dan ergo.test dan GB+ :0 Daniel Garcia")
        .server(":ergo.test 315 emily #papo :End of WHO list")
        .inspect(|session| {
            let dan = session.state().user("dan").unwrap();
            assert_eq!(dan.account(), &Account::Unknown);
            assert_eq!(dan.username(), Some("~dan"));
            assert_eq!(dan.host(), Some("account/dan"));
            assert_eq!(dan.realname(), Some("Daniel Garcia"));
            assert_eq!(dan.away(), Some(""));
            assert!(dan.is_bot());
        })
        .no_event();
}

#[test]
fn who_here_flags_clear_a_previous_away_state() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":dan!~dan@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":dan!~dan@cloak.example AWAY :Gone to lunch")
        .event(|event| {
            matches!(
                event,
                Event::AwayChanged {
                    message: Some(_),
                    ..
                }
            )
        })
        .server(":ergo.test 366 emily #papo :End of NAMES list")
        .client("MODE #papo")
        .client("WHO #papo")
        .event(|event| matches!(event, Event::ChannelSynced { .. }))
        .server(":ergo.test 352 emily #papo ~dan cloak.example ergo.test dan H :0 Daniel Garcia")
        .inspect(|session| assert_eq!(session.state().user("dan").unwrap().away(), None))
        .no_event();
}

#[test]
fn plain_who_recognizes_each_bot_flag_source() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily BOT=Z :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":ergo.test 366 emily #papo :End of NAMES list")
        .client("MODE #papo")
        .client("WHO #papo")
        .event(|event| matches!(event, Event::ChannelSynced { .. }))
        .server(":ergo.test 352 emily #papo bot services.example ergo.test service-bot HB :0 Service Bot")
        .server(":ergo.test 352 emily #papo bot users.example ergo.test relay-bot HZ :0 Relay Bot")
        .server(":ergo.test 352 emily #papo alice users.example ergo.test alice H :0 Alice Smith")
        .inspect(|session| {
            assert!(session.state().user("service-bot").unwrap().is_bot());
            assert!(session.state().user("relay-bot").unwrap().is_bot());
            assert!(!session.state().user("alice").unwrap().is_bot());
        })
        .no_event();
}

#[test]
fn whox_replies_require_our_token_and_an_active_sweep() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily WHOX :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":ergo.test 366 emily #papo :End of NAMES list")
        .client("MODE #papo")
        .client("WHO #papo %tcuhnfar,152")
        .event(|event| matches!(event, Event::ChannelSynced { .. }))
        .server(":ergo.test 354 emily 999 #papo dan cloak.example dan H dan :Daniel Garcia")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().raw_command() == "354"))
        .server(":ergo.test 315 emily #papo :End of WHO list")
        .server(":ergo.test 354 emily 152 #papo alice users.example alice H alice :Alice Smith")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().raw_command() == "354"))
        .inspect(|session| {
            assert!(session.state().user("dan").is_none());
            assert!(session.state().user("alice").is_none());
        })
        .no_event();
}

#[test]
fn user_requested_who_replies_remain_messages() {
    Transcript::new(Config::default().nick("emily"))
        .send(Message::new("WHO", ["#rust"]))
        .client("WHO #rust")
        .server(":ergo.test 352 emily #rust ferris user/ferris ergo.test ferris H :0 Ferris Crab")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().raw_command() == "352"))
        .server(":ergo.test 315 emily #rust :End of WHO list")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().raw_command() == "315"))
        .no_event();
}

#[test]
fn reports_parts_and_kicks_for_other_users_and_ourselves() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "emily"))
        .server(":dan!dan@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "dan"))
        .server(":dan!dan@cloak.example PART #papo :Leaving")
        .event(|event| matches!(event, Event::Parted { channel, nick, reason } if channel == "#papo" && nick == "dan" && reason.as_deref() == Some("Leaving")))
        .server(":dan!dan@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "dan"))
        .server(":chanop!op@staff.example KICK #papo dan :Rule violation")
        .event(|event| matches!(event, Event::Kicked { channel, nick, by, reason } if channel == "#papo" && nick == "dan" && by == "chanop" && reason.as_deref() == Some("Rule violation")))
        .server(":emily!em@127.0.0.1 PART #papo :Requested")
        .event(|event| matches!(event, Event::Parted { nick, .. } if nick == "emily"))
        .server(":emily!em@127.0.0.1 JOIN #rust")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "emily"))
        .server(":chanop!op@staff.example KICK #rust emily :Requested")
        .event(|event| matches!(event, Event::Kicked { nick, .. } if nick == "emily"))
        .no_event();
}

#[test]
fn reports_nick_changes_and_quit_channels_before_removal() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":emily!em@127.0.0.1 JOIN #rust")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":dan!dan@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":dan!dan@cloak.example JOIN #rust")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":dan!dan@cloak.example NICK :dan-away")
        .event(|event| matches!(event, Event::NickChanged { old, new } if old == "dan" && new == "dan-away"))
        .server(":emily!em@127.0.0.1 NICK :emily_")
        .event(|event| matches!(event, Event::NickChanged { old, new } if old == "emily" && new == "emily_"))
        .server(":dan-away!dan@cloak.example QUIT :Client exited")
        .event(|event| match event {
            Event::Quit { nick, reason, channels } => {
                let mut channels = channels.clone();
                channels.sort();
                nick == "dan-away" && reason.as_deref() == Some("Client exited")
                    && channels == ["#papo", "#rust"]
            }
            _ => false,
        })
        .no_event();
}

#[test]
fn reconnect_clears_channel_and_user_state() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":dan!dan@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .disconnect()
        .connect()
        .inspect(|session| {
            assert_eq!(session.state().channels().count(), 0);
            assert!(session.state().user("dan").is_none());
        })
        .ignore_rest();
}

#[test]
fn rekeys_existing_state_when_casemapping_changes() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily CASEMAPPING=ascii :are supported")
        .server(":emily!em@127.0.0.1 JOIN #Papo[dev]")
        .server(":Alice[bot]!alice@users.example JOIN #Papo[dev]")
        .inspect(|session| {
            assert!(session.state().channel("#papo{dev}").is_none());
            assert!(session.state().user("alice{bot}").is_none());
        })
        .server(":ergo.test 005 emily CASEMAPPING=rfc1459 :are supported")
        .inspect(|session| {
            let channel = session.state().channel("#papo{dev}").unwrap();
            assert!(channel.member("alice{bot}").is_some());
            assert!(session.state().user("alice{bot}").is_some());
        })
        .ignore_rest();
}
