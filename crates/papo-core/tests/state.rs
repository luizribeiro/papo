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
fn welcome_adopts_the_server_assigned_nick_in_state() {
    Transcript::new(Config::default().nick("oldnick"))
        .server(":irc.example 001 account :Welcome to the network")
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
        .server(":ergo.test 324 emily #papo +kln swordfish 10")
        .server(":ergo.test 329 emily #papo 1723501539")
        .server(":ergo.test 332 emily #papo :Welcome to the papo channel")
        .server(":ergo.test 333 emily #papo alice 1723501612")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.mode('k'), Some(Some("swordfish")));
            assert_eq!(channel.mode('l'), Some(Some("10")));
            assert_eq!(channel.mode('n'), Some(None));
            assert_eq!(channel.created_at(), Some(1_723_501_539));
            assert_eq!(channel.topic(), Some("Welcome to the papo channel"));
            assert_eq!(channel.topic_set_by(), Some("alice"));
            assert_eq!(channel.topic_set_at(), Some(1_723_501_612));
        })
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
        .server(":alice!alice@staff.example TOPIC #papo :Rust 1.90 is out")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.topic(), Some("Rust 1.90 is out"));
            assert_eq!(channel.topic_set_by(), Some("alice"));
            assert_eq!(channel.topic_set_at(), None);
        })
        .server(":alice!alice@staff.example TOPIC #papo :")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.topic(), None);
            assert_eq!(channel.topic_set_by(), Some("alice"));
        })
        .ignore_rest();
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
fn no_implicit_names_marks_our_join_synced_without_366() {
    Transcript::new(Config::default().wanted_caps(["no-implicit-names"]))
        .connect_default()
        .server("CAP * LS :draft/no-implicit-names")
        .client("CAP REQ :draft/no-implicit-names")
        .server("CAP * ACK :draft/no-implicit-names")
        .client("CAP END")
        .server(":papo!papo@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { channel, nick } if channel == "#papo" && nick == "papo"))
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
        .server(":papo!papo@127.0.0.1 JOIN #Papo")
        .event(|event| matches!(event, Event::Joined { channel, nick } if channel == "#Papo" && nick == "papo"))
        .inspect(|session| assert!(!session.state().channel("#papo").unwrap().is_synced()))
        .server(":ergo.test 366 papo #Papo :End of NAMES list")
        .event(|event| matches!(event, Event::ChannelSynced { channel } if channel == "#Papo"))
        .inspect(|session| assert!(session.state().channel("#papo").unwrap().is_synced()))
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
