//! Transcript tests for channel and user state.

mod support;

use std::time::{Duration, Instant, UNIX_EPOCH};

use papo_core::{
    Account, Authorship, Away, Config, Conversation, Event, ModeAction, ModeArg, ModeChange,
    ModeValue, Rank, RateLimit, ResponseMatcher, Secret, Source, StateLimit, Subject, WhoOnJoin,
};
use papo_proto::{AccountName, Casemapping, ChannelName, InvalidIdentifier, Line, Nick, Origin};
use support::Transcript;

fn prefix_symbols(prefixes: &[papo_core::Prefix]) -> Vec<char> {
    prefixes.iter().map(|prefix| prefix.symbol()).collect()
}

#[test]
fn account_and_away_accessors_preserve_unknown_states() {
    let account = Casemapping::Rfc1459.account("emily");
    assert_eq!(Account::Unknown.name(), None);
    assert_eq!(Account::LoggedOut.name(), None);
    assert_eq!(Account::LoggedIn(account.clone()).name(), Some(&account));

    assert_eq!(Away::Unknown.message(), None);
    assert_eq!(Away::Present.message(), None);
    assert_eq!(Away::__away_value(None).message(), None);
    assert_eq!(
        Away::__away_value(Some("reviewing the release".into())).message(),
        Some("reviewing the release")
    );
}

#[test]
fn starts_with_configured_identity_and_empty_shared_state() {
    let mut config = Config::default().nick("emily").username("em");
    config.origin = Origin::__new(73);
    Transcript::new(config)
        .inspect(|session| {
            let state = session.state();
            assert_eq!(state.nick(), "emily");
            assert_eq!(state.nick().casemapping(), Casemapping::Rfc1459);
            assert_eq!(state.nick().origin(), Origin::__new(73));
            assert_eq!(state.username(), "em");
            assert_eq!(state.host(), None);
            assert_eq!(state.account(), &Account::LoggedOut);
            assert_eq!(state.away(), &Away::Present);
        })
        .no_event();
}

#[test]
fn own_user_exists_without_membership_and_survives_part_and_nick() {
    Transcript::new(
        Config::default()
            .nick("emily")
            .username("em")
            .realname("Emily Example"),
    )
    .inspect(|session| {
        let own = session.state().user("emily").unwrap();
        assert_eq!(own.username(), Some("em"));
        assert_eq!(own.realname(), Some("Emily Example"));
        assert_eq!(own.channels().count(), 0);
    })
    .server(":emily!em@users/emily JOIN #papo")
    .server(":emily!em@users/emily PART #papo :Leaving")
    .server(":emily!em@users/emily NICK :emily-away")
    .inspect(|session| {
        assert!(session.state().user("emily").is_none());
        assert_eq!(
            session
                .state()
                .user("emily-away")
                .unwrap()
                .channels()
                .count(),
            0
        );
    })
    .ignore_rest();
}

#[test]
fn a_foreign_rename_into_the_own_nick_keeps_the_own_row() {
    Transcript::new(Config::default().nick("papo").username("papo"))
        .server(":papo!papo@localhost JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "papo"))
        .server(":ergo.test 353 papo = #papo :papo alice")
        .server(":ergo.test 366 papo #papo :End of /NAMES list.")
        .event_until(|event| matches!(event, Event::ChannelSynced { .. }))
        .inspect(|session| {
            assert_eq!(session.state().user("papo").unwrap().username(), Some("papo"));
        })
        .server(":alice NICK papo")
        .event(|event| matches!(event, Event::NickChanged { authorship: Authorship::Other, .. }))
        .inspect(|session| {
            assert_eq!(session.state().nick(), "papo");
            assert_eq!(session.state().username(), "papo");
        })
        .server(":ergo.test 396 papo cloak.example :is now your displayed host")
        .event(|event| matches!(event, Event::HostChanged { username, authorship: Authorship::Own, .. } if username == "papo"))
        .inspect(|session| {
            assert_eq!(session.state().username(), "papo");
            assert_eq!(session.state().host(), Some("cloak.example"));
        })
        .ignore_rest();
}

#[test]
fn own_quit_clears_shared_users_and_preserves_identity() {
    Transcript::new(
        Config::default()
            .nick("emily")
            .username("em")
            .realname("Emily Example"),
    )
    .server(":emily!em@users/emily JOIN #papo")
    .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "emily"))
    .server(":dan!dan@cloak.example JOIN #papo")
    .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "dan"))
    .server(":emily!em@users/emily QUIT :Client exited")
    .event(|event| matches!(event, Event::Quit { nick, .. } if nick == "emily"))
    .inspect(|session| {
        let state = session.state();
        assert_eq!(state.channels().count(), 0);
        assert!(state.user("dan").is_none());
        let own = state.user("emily").unwrap();
        assert_eq!(own.username(), Some("em"));
        assert_eq!(own.realname(), Some("Emily Example"));
        assert_eq!(own.channels().count(), 0);
    })
    .no_event();
}

fn assert_own_identity_matches(session: &papo_core::Session) {
    let state = session.state();
    let own = state.user(state.nick()).unwrap();
    assert_eq!(own.username(), Some(state.username()));
    assert_eq!(own.host(), state.host());
    assert_eq!(own.realname(), Some(state.realname()));
}

#[test]
fn own_identity_updates_have_one_user_row_before_and_after_membership() {
    Transcript::new(
        Config::default()
            .nick("emily")
            .username("em")
            .realname("Emily Example"),
    )
    .inspect(assert_own_identity_matches)
    .server(":ergo.test 396 emily account/emily :is now your displayed host")
    .event(|event| matches!(event, Event::HostChanged { host, .. } if host == "account/emily"))
    .inspect(assert_own_identity_matches)
    .server(":emily!em@account/emily CHGHOST ~em users/emily")
    .event(|event| matches!(event, Event::HostChanged { username, host, .. } if username == "~em" && host == "users/emily"))
    .inspect(assert_own_identity_matches)
    .server(":emily!~em@users/emily SETNAME :Emily Smith")
    .event(|event| matches!(event, Event::RealnameChanged { realname, .. } if realname == "Emily Smith"))
    .inspect(assert_own_identity_matches)
    .server(":emily!~em@users/emily JOIN #papo")
    .server(":emily!~em@users/emily PART #papo :Leaving")
    .server(":emily!~em@users/emily NICK :emily-away")
    .inspect(assert_own_identity_matches)
    .ignore_rest();
}

fn assert_own_presence_matches(session: &papo_core::Session) {
    let state = session.state();
    let own = state.user(state.nick()).unwrap();
    assert_eq!(own.account(), state.account());
    assert_eq!(own.away(), state.away());
}

#[test]
fn own_presence_updates_have_one_user_row_across_membership_and_nick() {
    Transcript::new(Config::default().nick("emily"))
        .inspect(|session| {
            assert_own_presence_matches(session);
            assert_eq!(session.state().account(), &Account::LoggedOut);
            assert_eq!(session.state().away(), &Away::Present);
        })
        .server(":ergo.test 306 emily :You have been marked as being away")
        .event(|event| {
            matches!(
                event,
                Event::AwayChanged {
                    away: Away::Away { .. },
                    ..
                }
            )
        })
        .inspect(|session| {
            assert_own_presence_matches(session);
            assert!(matches!(session.state().away(), Away::Away { .. }));
        })
        .server(":ergo.test 305 emily :You are no longer marked as being away")
        .event(|event| {
            matches!(
                event,
                Event::AwayChanged {
                    away: Away::Present,
                    ..
                }
            )
        })
        .inspect(|session| {
            assert_own_presence_matches(session);
            assert_eq!(session.state().away(), &Away::Present);
        })
        .server(":emily!~em@users/emily ACCOUNT emily")
        .event(|event| {
            matches!(
                event,
                Event::AccountChanged {
                    account: Account::LoggedIn(_),
                    ..
                }
            )
        })
        .inspect(|session| {
            assert_own_presence_matches(session);
            assert!(matches!(session.state().account(), Account::LoggedIn(_)));
        })
        .server(":emily!~em@users/emily ACCOUNT *")
        .event(|event| {
            matches!(
                event,
                Event::AccountChanged {
                    account: Account::LoggedOut,
                    ..
                }
            )
        })
        .inspect(|session| {
            assert_own_presence_matches(session);
            assert_eq!(session.state().account(), &Account::LoggedOut);
        })
        .server(":ergo.test 900 emily emily!~em@users/emily emily :You are now logged in as emily")
        .inspect(|session| {
            assert_own_presence_matches(session);
            assert!(matches!(session.state().account(), Account::LoggedIn(_)));
        })
        .server(":ergo.test 901 emily emily!~em@users/emily :You are now logged out")
        .inspect(|session| {
            assert_own_presence_matches(session);
            assert_eq!(session.state().account(), &Account::LoggedOut);
        })
        .server(":emily!~em@users/emily JOIN #papo")
        .server(":emily!~em@users/emily PART #papo :Leaving")
        .server(":emily!~em@users/emily NICK :emily-away")
        .inspect(assert_own_presence_matches)
        .ignore_rest();
}

#[test]
fn a_server_without_accounts_keeps_our_entry_logged_out() {
    Transcript::new(Config::default().wanted_caps(std::iter::empty::<&str>()))
        .connect_default()
        .server("CAP * LS :")
        .client("CAP END")
        .welcome(":irc.example 001 papo :Welcome to the Internet Relay Network")
        .inspect(|session| assert_eq!(session.state().account(), &Account::LoggedOut))
        .no_event();
}

#[test]
fn parses_targets_with_advertised_status_and_channel_types() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily CHANTYPES=#! STATUSMSG=@+ :are supported")
        .inspect(|session| {
            let state = session.state();
            assert!(matches!(
                state.parse_target("@#ops"),
                Ok((Some(status), Conversation::Channel(channel)))
                    if status.symbol() == '@' && channel == "#ops"
            ));
            assert!(matches!(
                state.parse_target("!staff"),
                Ok((None, Conversation::Channel(channel))) if channel == "!staff"
            ));
            assert_eq!(
                state.parse_target("$*.example.org"),
                Err(InvalidIdentifier::InvalidPrefix('$'))
            );
            assert!(matches!(
                state.parse_target("emily"),
                Ok((None, Conversation::Private(nick))) if nick == "emily"
            ));
            assert_eq!(
                state.parse_target("*"),
                Err(InvalidIdentifier::InvalidCharacter('*'))
            );
        })
        .no_event();
}

#[test]
fn rejects_invalid_targets_and_unadvertised_status_prefixes() {
    Transcript::new(Config::default().nick("emily"))
        .server(":solanum.example 005 emily CHANTYPES=# STATUSMSG=+ :are supported")
        .inspect(|session| {
            let state = session.state();
            assert_eq!(state.parse_target(""), Err(InvalidIdentifier::Empty));
            assert_eq!(
                state.parse_target("$*.example.org,alice"),
                Err(InvalidIdentifier::InvalidCharacter(','))
            );
            assert_eq!(
                state.parse_target("@#ops"),
                Err(InvalidIdentifier::InvalidPrefix('@'))
            );
            assert!(matches!(
                state.parse_target("+#ops"),
                Ok((Some(status), Conversation::Channel(channel)))
                    if status.symbol() == '+' && channel == "#ops"
            ));
        })
        .no_event();
}

#[test]
fn conversations_preserve_spelling_and_use_server_comparison_rules() {
    Transcript::new(Config::default().nick("papo"))
        .server(":ergo.test 005 papo CASEMAPPING=rfc1459 NICKLEN=32 STATUSMSG=@ CHANTYPES=# :are supported")
        .inspect(|session| {
            let conversation = session.state().conversation("Alice[bot]").unwrap();
            assert_eq!(conversation, "alice{bot}");
            assert_eq!(conversation.to_string(), "Alice[bot]");
            assert_eq!(
                session.state().conversation("@#papo"),
                Err(InvalidIdentifier::InvalidPrefix('@'))
            );
        })
        .no_event();
}

#[test]
fn seeded_rules_apply_offline_then_the_connection_uses_the_protocol_default() {
    Transcript::new(
        Config::default()
            .nick("emily[bot]")
            .casemapping(Casemapping::Ascii),
    )
    .inspect(|session| {
        assert_eq!(session.state().casemapping(), Casemapping::Ascii);
        assert_ne!(session.state().nick(), "emily{bot}");
        let Ok((None, Conversation::Private(nick))) = session.state().parse_target("Alice[x]")
        else {
            panic!("expected a private conversation")
        };
        assert_eq!(nick.casemapping(), Casemapping::Ascii);
    })
    .connect()
    .inspect(|session| {
        assert_eq!(session.state().casemapping(), Casemapping::Rfc1459);
        assert_eq!(session.state().nick(), "emily{bot}");
    })
    .ignore_rest();
}

#[test]
fn disconnect_retains_the_last_advertised_rules() {
    Transcript::new(Config::default().nick("emily[bot]"))
        .server(":irc.example 005 emily CASEMAPPING=ascii CHANTYPES=! STATUSMSG=@ :are supported")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .disconnect()
        .inspect(|session| {
            assert_eq!(session.state().casemapping(), Casemapping::Ascii);
            assert_ne!(session.state().nick(), "emily{bot}");
            assert!(matches!(
                session.state().parse_target("@!ops"),
                Ok((Some(status), Conversation::Channel(channel)))
                    if status.symbol() == '@' && channel == "!ops"
            ));
        })
        .no_event();
}

#[test]
fn reports_one_rules_change_against_the_remembered_connection() {
    Transcript::new(
        Config::default()
            .nick("emily")
            .casemapping(Casemapping::Ascii),
    )
    .connect()
    .server(":irc.example 005 emily CASEMAPPING=ascii :are supported")
    .no_event()
    .disconnect()
    .connect()
    .server(":irc.example 005 emily CASEMAPPING=rfc1459-strict :are supported")
    .event(|event| {
        matches!(
            event,
            Event::CasemappingChanged {
                old: Casemapping::Ascii,
                new: Casemapping::Rfc1459Strict,
                ..
            }
        )
    })
    .server(":irc.example 005 emily CASEMAPPING=rfc1459 :are supported")
    .no_event()
    .disconnect()
    .inspect(|session| assert_eq!(session.state().casemapping(), Casemapping::Rfc1459))
    .ignore_rest();
}

#[test]
fn welcome_adopts_the_server_assigned_nick_in_state() {
    Transcript::new(Config::default().nick("oldnick"))
        .welcome(":irc.example 001 account :Welcome to the network")
        .inspect(|session| assert_eq!(session.state().nick(), "account"))
        .no_event();
}

#[test]
fn session_nick_tracks_own_and_server_forced_changes_with_casemapping() {
    Transcript::new(Config::default().nick("emily[x]"))
        .server(":irc.example 005 emily[x] CASEMAPPING=ascii :are supported")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .welcome(":irc.example 001 emily[x] :Welcome to the network")
        .server(":EMILY[X]!em@localhost NICK :emily{ops}")
        .event(|event| {
            matches!(event, Event::NickChanged { old, new, .. } if old == "EMILY[X]" && new == "emily{ops}")
        })
        .inspect(|session| {
            assert_eq!(session.nick(), "emily{ops}");
            assert_eq!(session.state().nick(), "emily{ops}");
        })
        .server(":EMILY{OPS}!em@localhost NICK :emily-forced")
        .event(|event| {
            matches!(event, Event::NickChanged { old, new, .. } if old == "EMILY{OPS}" && new == "emily-forced")
        })
        .inspect(|session| {
            assert_eq!(session.nick(), "emily-forced");
            assert_eq!(session.state().nick(), "emily-forced");
        })
        .no_event();
}

#[test]
fn silent_labelled_response_updates_state_without_a_state_event() {
    Transcript::new(Config::default().wanted_caps(["labeled-response"]))
        .connect_default()
        .server("CAP * LS :labeled-response")
        .client("CAP REQ :labeled-response")
        .server("CAP * ACK :labeled-response")
        .client("CAP END")
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 422 papo :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
        .update(|session| {
            session
                .request_silent(
                    std::time::Instant::now(),
                    Line::new("NICK").with_params(["papo-renamed"]),
                )
                .unwrap();
        })
        .client("@label=0 NICK papo-renamed")
        .server("@label=0 :papo!papo@localhost NICK :papo-renamed")
        .event(|event| matches!(event, Event::Response(response) if response.lines.len() == 1))
        .inspect(|session| assert_eq!(session.state().nick(), "papo-renamed"))
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
fn typed_state_lookups_refuse_names_from_another_client() {
    let local = Origin::__new(73);
    let foreign = Origin::__new(91);
    let mut config = Config::default().nick("emily");
    config.origin = local;
    Transcript::new(config)
        .server(":emily!em@cloak.example JOIN #papo")
        .server(":dan!dan@cloak.example JOIN #papo")
        .inspect(|session| {
            let state = session.state();
            let channel = local.__channel(Casemapping::Rfc1459, "#papo");
            let nick = local.__nick(Casemapping::Rfc1459, "dan");
            assert!(state.channel(&channel).is_some());
            assert!(state.user(&nick).is_some());
            assert!(state.channel("#PAPO").unwrap().member("DAN").is_some());

            let channel = foreign.__channel(Casemapping::Rfc1459, "#papo");
            let nick = foreign.__nick(Casemapping::Rfc1459, "dan");
            assert!(state.channel(&channel).is_none());
            assert!(state.user(&nick).is_none());
            assert!(state.channel("#papo").unwrap().member(&nick).is_none());
        })
        .ignore_rest();
}

#[test]
fn read_models_expose_typed_identifiers() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo emily :Emily")
        .server(":dan!~dan@cloak.example JOIN #papo dan :Daniel Garcia")
        .inspect(|session| {
            let own_nick: &Nick = session.state().nick();
            let channel_query = session.isupport().casemapping().channel("#PAPO");
            let user_query = session.isupport().casemapping().nick("DAN");
            let channel = session.state().channel(&channel_query).unwrap();
            let channel_name: &ChannelName = channel.name();
            let member_nick: &Nick = channel.member(&user_query).unwrap().nick();
            let user = session.state().user(&user_query).unwrap();
            let user_nick: &Nick = user.nick();
            let shared_channel: &ChannelName = user.channels().next().unwrap();
            let Account::LoggedIn(account) = user.account() else {
                panic!("extended join should report the account");
            };
            let account: &AccountName = account;

            assert_eq!(session.state().channels().len(), 1);
            assert_eq!(channel.members().len(), 2);
            assert_eq!(user.channels().len(), 1);
            assert_eq!(session.state().monitored().len(), 0);
            assert_eq!(session.state().metadata_subscriptions().len(), 0);
            assert_eq!(own_nick, "emily");
            assert_eq!(channel_name, "#papo");
            assert_eq!(member_nick, "dan");
            assert_eq!(user_nick, "dan");
            assert_eq!(shared_channel, "#papo");
            assert_eq!(account, "dan");
        })
        .ignore_rest();
}

#[test]
fn wire_names_capture_the_advertised_ascii_casemapping() {
    Transcript::new(Config::default().nick("emily[bot]"))
        .server(":ergo.test 005 emily[bot] CASEMAPPING=ascii :are supported")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .server(":Emily[bot]!em@127.0.0.1 JOIN #Papo[dev] Emily[bot] :Emily Example")
        .event(|event| {
            matches!(
                event,
                Event::Joined { channel, nick, .. }
                    if channel.casemapping() == Casemapping::Ascii
                        && nick.casemapping() == Casemapping::Ascii
                        && channel != "#papo{dev}"
                        && nick != "emily{bot}"
            )
        })
        .inspect(|session| {
            let channel = session.state().channel("#papo[dev]").unwrap();
            let user = session.state().user("EMILY[bot]").unwrap();
            let Account::LoggedIn(account) = user.account() else {
                panic!("extended join should report the account");
            };

            assert_eq!(channel.name().as_ref(), "#Papo[dev]");
            assert_eq!(channel.name().casemapping(), Casemapping::Ascii);
            assert_eq!(
                channel.member("emily[bot]").unwrap().nick().casemapping(),
                Casemapping::Ascii
            );
            assert_eq!(user.nick().casemapping(), Casemapping::Ascii);
            assert_eq!(account.casemapping(), Casemapping::Ascii);
        })
        .ignore_rest();
}

#[test]
fn state_change_events_capture_the_advertised_ascii_casemapping() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily CASEMAPPING=ascii :are supported")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .server(":emily!em@127.0.0.1 JOIN #Papo[dev]")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":Alice[bot]!alice@staff.example JOIN #Papo[dev]")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":Alice[bot]!alice@staff.example ACCOUNT Alice[acct]")
        .event(|event| matches!(event, Event::AccountChanged { nick, account: Account::LoggedIn(account), .. } if nick.casemapping() == Casemapping::Ascii && account.casemapping() == Casemapping::Ascii))
        .server(":Alice[bot]!alice@staff.example AWAY :Reviewing logs")
        .event(|event| matches!(event, Event::AwayChanged { nick, .. } if nick.casemapping() == Casemapping::Ascii && nick != "alice{bot}"))
        .server(":Alice[bot]!alice@staff.example CHGHOST alice users/alice")
        .event(|event| matches!(event, Event::HostChanged { nick, .. } if nick.casemapping() == Casemapping::Ascii && nick != "alice{bot}"))
        .server(":Alice[bot]!alice@users/alice SETNAME :Alice Smith")
        .event(|event| matches!(event, Event::RealnameChanged { nick, .. } if nick.casemapping() == Casemapping::Ascii && nick != "alice{bot}"))
        .server(":Alice[bot]!alice@users/alice TOPIC #Papo[dev] :Release planning")
        .event(|event| matches!(event, Event::TopicChanged { channel, .. } if channel.casemapping() == Casemapping::Ascii && channel != "#papo{dev}"))
        .no_event();
}

#[cfg(feature = "precis")]
#[test]
fn utf8mapping_builds_names_with_precis_rules() {
    Transcript::new(Config::default().nick("émilie"))
        .server(":ergo.test 005 émilie CASEMAPPING=ascii UTF8MAPPING=rfc8265 :are supported")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .server(":ÉMILIE!em@127.0.0.1 JOIN #Papo ÉMILIE :Émilie Example")
        .event(|event| {
            matches!(
                event,
                Event::Joined { channel, nick, .. }
                    if channel.casemapping() == Casemapping::Precis
                        && nick.casemapping() == Casemapping::Precis
            )
        })
        .inspect(|session| {
            let user = session.state().user("émilie").unwrap();
            assert_eq!(session.state().casemapping(), Casemapping::Precis);
            assert_eq!(user.nick().casemapping(), Casemapping::Precis);
            assert_eq!(user.nick(), "émilie");
        })
        .ignore_rest();
}

#[test]
fn names_received_before_isupport_are_refolded_under_announced_rules() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #Papo[dev]")
        .event(|event| {
            matches!(
                event,
                Event::Joined { channel, nick, .. }
                    if channel.casemapping() == Casemapping::Rfc1459
                        && nick.casemapping() == Casemapping::Rfc1459
                        && channel == "#papo{dev}"
            )
        })
        .inspect(|session| assert!(session.state().channel("#papo{dev}").is_some()))
        .server(":ergo.test 005 emily CASEMAPPING=ascii :are supported")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .inspect(|session| {
            let channel = session.state().channel("#papo[dev]").unwrap();
            assert_eq!(channel.name().casemapping(), Casemapping::Ascii);
            assert!(session.state().channel("#papo{dev}").is_none());
        })
        .no_event();
}

#[test]
fn exposes_our_host_and_original_channel_metadata() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #Papo")
        .event(|event| matches!(event, Event::Joined { channel, nick, .. } if channel == "#Papo" && nick == "emily"))
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
                &Account::LoggedIn(Casemapping::default().account("dan"))
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
        .event_until(|event| matches!(event, Event::AccountChanged { actor, nick, account, .. } if actor == "dan!~dan@cloak.example" && nick == "dan" && account == &Account::LoggedIn(Casemapping::default().account("dan"))))
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().account(),
                &Account::LoggedIn(Casemapping::default().account("dan"))
            );
        })
        .server(":dan!~dan@cloak.example ACCOUNT *")
        .event(|event| matches!(event, Event::AccountChanged { nick, account, .. } if nick == "dan" && account == &Account::LoggedOut))
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
        .event_until(|event| matches!(event, Event::AccountChanged { nick, account, .. } if nick == "dan" && account == &Account::LoggedIn(Casemapping::default().account("dan"))))
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().account(),
                &Account::LoggedIn(Casemapping::default().account("dan"))
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
        .event_until(|event| matches!(event, Event::AwayChanged { actor, nick, away: Away::Away { message: Some(message), .. }, .. } if actor == "dan!~dan@cloak.example" && nick == "dan" && message == "Gone to lunch"))
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().away(),
                &Away::__away_value(Some("Gone to lunch".into()))
            );
        })
        .server(":dan!~dan@cloak.example AWAY")
        .event(|event| matches!(event, Event::AwayChanged { nick, away: Away::Present, .. } if nick == "dan"))
        .inspect(|session| assert_eq!(session.state().user("dan").unwrap().away(), &Away::Present))
        .ignore_rest();
}

#[test]
fn away_notify_separates_our_state_and_suppresses_repeated_messages() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":dan!~dan@cloak.example JOIN #papo")
        .server(":dan!~dan@cloak.example AWAY :Gone to lunch")
        .event_until(|event| matches!(event, Event::AwayChanged { nick, away: Away::Away { message: Some(message), .. }, authorship: Authorship::Other, .. } if nick == "dan" && message == "Gone to lunch"))
        .inspect(|session| {
            assert_eq!(session.state().away(), &Away::Present);
            assert_eq!(
                session.state().user("dan").unwrap().away(),
                &Away::__away_value(Some("Gone to lunch".into()))
            );
        })
        .server(":emily!em@127.0.0.1 AWAY :On call")
        .event(|event| matches!(event, Event::AwayChanged { nick, away: Away::Away { message: Some(message), .. }, .. } if nick == "emily" && message == "On call"))
        .inspect(|session| {
            assert_eq!(
                session.state().away(),
                &Away::__away_value(Some("On call".into()))
            );
        })
        .server(":emily!em@127.0.0.1 AWAY :On call")
        .no_event()
        .server(":emily!em@127.0.0.1 AWAY :Reviewing alerts")
        .event(|event| matches!(event, Event::AwayChanged { nick, away: Away::Away { message: Some(message), .. }, .. } if nick == "emily" && message == "Reviewing alerts"))
        .inspect(|session| {
            assert_eq!(
                session.state().away(),
                &Away::__away_value(Some("Reviewing alerts".into()))
            );
        })
        .ignore_rest();
}

#[test]
fn away_notify_star_records_an_unspecified_away_state() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":dan!~dan@cloak.example JOIN #papo dan :Daniel Garcia")
        .server(":dan!~dan@cloak.example AWAY *")
        .event_until(|event| matches!(event, Event::AwayChanged { nick, away: Away::Away { message: None, .. }, .. } if nick == "dan"))
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().away(),
                &Away::__away_value(None)
            );
        })
        .ignore_rest();
}

#[test]
fn away_notify_immediately_after_join_records_initial_state() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":alice!alice@staff.example JOIN #papo alice :Alice Smith")
        .inspect(|session| {
            assert_eq!(
                session.state().user("alice").unwrap().away(),
                &Away::Unknown
            );
        })
        .server(":alice!alice@staff.example AWAY :Working")
        .inspect(|session| {
            assert_eq!(
                session.state().user("alice").unwrap().away(),
                &Away::__away_value(Some("Working".into()))
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
        .event_until(|event| matches!(event, Event::AwayChanged { nick, away: Away::Away { message: Some(message), .. }, .. } if nick == "dan" && message == "Gone to lunch"))
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().away(),
                &Away::__away_value(Some("Gone to lunch".into()))
            );
        })
        .ignore_rest();
}

#[test]
fn uncorrelated_away_numerics_report_an_unspecified_message() {
    Transcript::new(Config::default().nick("emily"))
        .inspect(|session| assert_eq!(session.state().away(), &Away::Present))
        .server(":ergo.test 306 emily :You have been marked as being away")
        .event(|event| matches!(event, Event::AwayChanged { nick, away: Away::Away { message: None, .. }, authorship: Authorship::Own, .. } if nick == "emily"))
        .inspect(|session| {
            assert_eq!(session.state().away(), &Away::__away_value(None));
        })
        .server(":ergo.test 305 emily :You are no longer marked as being away")
        .event(|event| matches!(event, Event::AwayChanged { nick, away: Away::Present, authorship: Authorship::Own, .. } if nick == "emily"))
        .inspect(|session| assert_eq!(session.state().away(), &Away::Present))
        .no_event();
}

#[test]
fn own_away_confirmation_carries_the_requested_message() {
    Transcript::new(Config::default().nick("emily"))
        .update(|session| {
            session
                .request_with_matcher(
                    Instant::now(),
                    Line::new("AWAY").with_params(["Reviewing the release notes"]),
                    ResponseMatcher::Away(true),
                )
                .unwrap();
        })
        .client("AWAY :Reviewing the release notes")
        .server(":ergo.test 306 emily :You have been marked as being away")
        .event(|event| matches!(event, Event::AwayChanged { nick, away: Away::Away { message: Some(message), .. }, .. } if nick == "emily" && message == "Reviewing the release notes"))
        .inspect(|session| {
            assert_eq!(
                session.state().away(),
                &Away::__away_value(Some("Reviewing the release notes".into()))
            );
        })
        .ignore_rest();
}

#[test]
fn away_numerics_do_not_repeat_our_own_state_changes() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 306 emily :You have been marked as being away")
        .event(|event| {
            matches!(
                event,
                Event::AwayChanged {
                    away: Away::Away { message: None, .. },
                    ..
                }
            )
        })
        .server(":ergo.test 306 emily :You have been marked as being away")
        .no_event()
        .inspect(|session| {
            assert_eq!(session.state().away(), &Away::__away_value(None));
        })
        .server(":ergo.test 305 emily :You are no longer marked as being away")
        .event(|event| {
            matches!(
                event,
                Event::AwayChanged {
                    away: Away::Present,
                    ..
                }
            )
        })
        .server(":ergo.test 305 emily :You are no longer marked as being away")
        .no_event()
        .inspect(|session| assert_eq!(session.state().away(), &Away::Present))
        .no_event();
}

#[test]
fn chghost_updates_shared_and_own_userhosts() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":dan!~dan@cloak.example JOIN #papo")
        .server(":dan!~dan@cloak.example CHGHOST dan account/dan")
        .event_until(|event| matches!(event, Event::HostChanged { actor, nick, username, host, .. } if actor == "dan!~dan@cloak.example" && nick == "dan" && username == "dan" && host == "account/dan"))
        .server(":emily!em@127.0.0.1 CHGHOST ~em users/emily")
        .event(|event| matches!(event, Event::HostChanged { nick, username, host, .. } if nick == "emily" && username == "~em" && host == "users/emily"))
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
        .event(|event| matches!(event, Event::HostChanged { nick, username, host, .. } if nick == "emily" && username == "~em" && host == "gateway/tor-sasl"))
        .inspect(|session| assert_eq!(session.state().host(), Some("gateway/tor-sasl")))
        .no_event();
}

#[test]
fn setname_updates_other_and_own_realnames() {
    Transcript::new(Config::default().nick("emily").realname("Emily Garcia"))
        .server(":emily!em@127.0.0.1 JOIN #papo emily :Emily Garcia")
    .server(":dan!~dan@cloak.example JOIN #papo dan :Daniel Garcia")
    .server(":dan!~dan@cloak.example SETNAME :Dan Garcia")
    .event_until(|event| matches!(event, Event::RealnameChanged { actor, nick, realname, .. } if actor == "dan!~dan@cloak.example" && nick == "dan" && realname == "Dan Garcia"))
    .server(":emily!em@127.0.0.1 SETNAME :Emily Smith")
    .event(|event| matches!(event, Event::RealnameChanged { nick, realname, .. } if nick == "emily" && realname == "Emily Smith"))
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
        .event(|event| matches!(event, Event::Joined { channel, nick, .. } if channel == "#papo" && nick == "dan"))
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
            |event| matches!(event, Event::ChannelSynced { channel, .. } if channel == "#papo"),
        )
        .client("MODE #papo")
        .client("WHO #papo")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert!(channel.is_synced());
            assert_eq!(channel.members().count(), 3);
            let emily = channel.member("emily").unwrap();
            assert_eq!(prefix_symbols(emily.prefixes()), ['@', '+']);
            assert_eq!(emily.rank(), Some(Rank::Op));
            assert!(emily.has(Rank::Op));
            assert!(emily.has(Rank::Voice));
            assert_eq!(channel.member("alice").unwrap().rank(), Some(Rank::Voice));
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
                prefix_symbols(
                    session
                        .state()
                        .channel("#papo")
                        .unwrap()
                        .member("emily")
                        .unwrap()
                        .prefixes()
                ),
                ['@', '+']
            );
        })
        .no_event();
}

#[test]
fn each_member_prefix_list_uses_the_table_active_when_built() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily PREFIX=(ov)@+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":ergo.test 353 emily = #papo :+@alice!a@cloak.example")
        .server(":ergo.test 005 emily PREFIX=(vo)+@ :are supported")
        .server(":ergo.test 353 emily = #papo :@+bob!b@cloak.example")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(
                prefix_symbols(channel.member("alice").unwrap().prefixes()),
                ['@', '+']
            );
            assert_eq!(
                prefix_symbols(channel.member("bob").unwrap().prefixes()),
                ['+', '@']
            );
        })
        .no_event();
}

#[test]
fn solanum_prefix_without_halfop_reports_only_advertised_ranks() {
    Transcript::new(Config::default().nick("emily"))
        .server(":solanum.example 005 emily PREFIX=(ov)@+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":solanum.example 353 emily = #papo :@+emily +dan")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            let emily = channel.member("emily").unwrap();
            assert_eq!(prefix_symbols(emily.prefixes()), ['@', '+']);
            assert_eq!(emily.rank(), Some(Rank::Op));
            assert!(emily.has(Rank::Op));
            assert!(emily.has(Rank::Voice));
            assert!(!emily.has(Rank::Halfop));
            assert_eq!(channel.member("dan").unwrap().rank(), Some(Rank::Voice));
        })
        .ignore_rest();
}

#[test]
fn inspircd_custom_prefix_does_not_invent_a_standard_rank() {
    Transcript::new(Config::default().nick("emily"))
        .server(":inspircd.example 005 emily PREFIX=(Yov)!@+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":inspircd.example 353 emily = #papo :@emily !service")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            let service = channel.member("service").unwrap();
            assert_eq!(prefix_symbols(service.prefixes()), ['!']);
            assert_eq!(service.rank(), None);
            assert!(!service.has(Rank::Owner));
            assert_eq!(channel.member("emily").unwrap().rank(), Some(Rank::Op));
        })
        .ignore_rest();
}

#[test]
fn unreal_prefix_modes_map_owner_and_halfop_by_letter() {
    Transcript::new(Config::default().nick("emily"))
        .server(":irc.unreal.example 005 emily PREFIX=(qaohv)~&@%+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":irc.unreal.example 353 emily = #papo :~emily &alice %dan")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            let emily = channel.member("emily").unwrap();
            assert_eq!(emily.rank(), Some(Rank::Owner));
            assert!(emily.has(Rank::Owner));
            assert!(!emily.has(Rank::Admin));
            assert_eq!(channel.member("alice").unwrap().rank(), Some(Rank::Admin));
            assert_eq!(channel.member("dan").unwrap().rank(), Some(Rank::Halfop));
        })
        .ignore_rest();
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
            assert_eq!(prefix_symbols(channel.member("dan").unwrap().prefixes()), ['@']);
            assert_eq!(prefix_symbols(channel.member("emily").unwrap().prefixes()), ['+']);
        })
        .server(":alice!alice@staff.example MODE #papo +v-o dan dan")
        .inspect(|session| {
            assert_eq!(
                prefix_symbols(session
                    .state()
                    .channel("#papo")
                    .unwrap()
                    .member("dan")
                    .unwrap()
                    .prefixes()),
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
            assert_eq!(channel.mode('k'), Some(ModeValue::Set));
            assert_eq!(channel.mode('k').unwrap().value(), None);
            assert_eq!(channel.key().map(Secret::expose), Some("swordfish"));
            assert!(!format!("{:?}", session.state()).contains("swordfish"));
            assert_eq!(channel.mode('l'), Some(ModeValue::Value("10")));
            assert_eq!(channel.mode('n'), Some(ModeValue::Set));
            assert_eq!(channel.mode('n').unwrap().value(), None);
            assert_eq!(
                channel.modes().find(|(mode, _)| *mode == 'k'),
                Some(('k', ModeValue::Set))
            );
            assert_eq!(
                channel.modes().find(|(mode, _)| *mode == 'l'),
                Some(('l', ModeValue::Value("10")))
            );
            assert_eq!(
                channel.modes().find(|(mode, _)| *mode == 'n'),
                Some(('n', ModeValue::Set))
            );
            assert_eq!(
                channel.created_at(),
                Some(UNIX_EPOCH + Duration::from_secs(1_723_501_539))
            );
            assert_eq!(channel.topic(), Some("Welcome to the papo channel"));
            assert!(
                channel
                    .topic_set_by()
                    .and_then(Source::nick)
                    .is_some_and(|nick| nick == "alice")
            );
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
            assert_eq!(channel.mode('m'), Some(ModeValue::Set));
            assert_eq!(channel.mode('b'), None);
        })
        .server(":ergo.test 324 emily #papo +n")
        .inspect(|session| assert_eq!(session.state().channel("#papo").unwrap().modes().count(), 1))
        .ignore_rest();
}

#[test]
fn parses_topic_setters_as_nicks_user_prefixes_and_servers() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":ergo.test 333 emily #papo alice 1723501612")
        .inspect(|session| {
            assert!(matches!(
                session.state().channel("#papo").unwrap().topic_set_by(),
                Some(Source::User { nick, username: None, host: None, .. }) if nick == "alice"
            ));
        })
        .server(":ergo.test 333 emily #papo alice!~a@staff.example 1723501613")
        .inspect(|session| {
            assert!(matches!(
                session.state().channel("#papo").unwrap().topic_set_by(),
                Some(Source::User { nick, username: Some(username), host: Some(host), .. })
                    if nick == "alice" && username == "~a" && host == "staff.example"
            ));
        })
        .server(":solanum.example 333 emily #papo irc.libera.chat 1760529342")
        .inspect(|session| {
            assert!(matches!(
                session.state().channel("#papo").unwrap().topic_set_by(),
                Some(Source::Server { name, .. }) if name == "IRC.LIBERA.CHAT"
            ));
        })
        .no_event();
}

#[test]
fn tracks_live_topic_changes_and_clearing() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server("@time=2026-10-22T13:14:15.250Z :alice!alice@staff.example TOPIC #papo :Rust 1.90 is out")
        .event(|event| matches!(event, Event::TopicChanged { channel, topic, actor, .. } if channel == "#papo" && topic.as_deref() == Some("Rust 1.90 is out") && actor.nick().is_some_and(|nick| nick == "alice")))
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.topic(), Some("Rust 1.90 is out"));
            assert!(matches!(
                channel.topic_set_by(),
                Some(Source::User { nick, username: Some(username), host: Some(host), .. })
                    if nick == "alice" && username == "alice" && host == "staff.example"
            ));
            assert_eq!(
                channel.topic_set_at(),
                papo_proto::time::parse("2026-10-22T13:14:15.250Z")
            );
        })
        .server("@time=2026-10-22T13:15:00.000Z :alice!alice@staff.example TOPIC #papo :")
        .event(|event| matches!(event, Event::TopicChanged { channel, topic, actor, .. } if channel == "#papo" && topic.is_none() && actor.nick().is_some_and(|nick| nick == "alice")))
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.topic(), None);
            assert!(channel
                .topic_set_by()
                .and_then(Source::nick)
                .is_some_and(|nick| nick == "alice"));
            assert_eq!(
                channel.topic_set_at(),
                papo_proto::time::parse("2026-10-22T13:15:00.000Z")
            );
        })
        .no_event();
}

#[test]
fn topic_events_and_state_keep_user_server_and_bare_nick_sources() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":alice!~a@host.example TOPIC #papo :Rust 1.90 is out")
        .event(|event| {
            matches!(
                event,
                Event::TopicChanged {
                    actor: Source::User {
                        nick,
                        username: Some(username),
                        host: Some(host),
                        ..
                    },
                    ..
                } if nick == "alice" && username == "~a" && host == "host.example"
            )
        })
        .inspect(|session| {
            assert!(matches!(
                session.state().channel("#papo").unwrap().topic_set_by(),
                Some(Source::User {
                    nick,
                    username: Some(username),
                    host: Some(host),
                    ..
                }) if nick == "alice" && username == "~a" && host == "host.example"
            ));
        })
        .server(":ergo.test TOPIC #papo :Scheduled maintenance")
        .event(|event| {
            matches!(
                event,
                Event::TopicChanged {
                    actor: Source::Server { name, .. },
                    ..
                } if name == "ERGO.TEST"
            )
        })
        .inspect(|session| {
            assert!(matches!(
                session.state().channel("#papo").unwrap().topic_set_by(),
                Some(Source::Server { name, .. }) if name == "ERGO.TEST"
            ));
        })
        .server(":ChanServ TOPIC #papo :Welcome to #papo")
        .event(|event| {
            matches!(
                event,
                Event::TopicChanged {
                    actor: Source::User {
                        nick,
                        username: None,
                        host: None,
                        ..
                    },
                    ..
                } if nick == "ChanServ"
            )
        })
        .inspect(|session| {
            assert!(matches!(
                session.state().channel("#papo").unwrap().topic_set_by(),
                Some(Source::User {
                    nick,
                    username: None,
                    host: None,
                    ..
                }) if nick == "ChanServ"
            ));
        })
        .no_event();
}

#[test]
fn reports_mixed_channel_and_user_mode_changes() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily CHANMODES=Ibe,k,fl,CEMRUimnstu PREFIX=(ov)@+ :are supported")
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":alice!alice@staff.example MODE #papo +ob-v alice *!*@spam.example bob")
        .event(|event| match event {
            Event::ModeChanged { subject, actor, changes, .. } => {
                matches!(subject, Subject::Channel(channel) if channel == "#papo")
                    && actor.nick().is_some_and(|nick| nick == "alice")
                    && changes == &vec![
                        ModeChange { mode: 'o', action: ModeAction::Add, arg: Some(ModeArg::__member(Casemapping::Rfc1459.nick("alice"), papo_core::Prefix::__new('o', '@'))) },
                        ModeChange { mode: 'b', action: ModeAction::Add, arg: Some(ModeArg::Mask(Casemapping::Rfc1459.mask("*!*@spam.example"))) },
                        ModeChange { mode: 'v', action: ModeAction::Remove, arg: Some(ModeArg::__member(Casemapping::Rfc1459.nick("bob"), papo_core::Prefix::__new('v', '+'))) },
                    ]
            }
            _ => false,
        })
        .server(":ChanServ!service@services.example MODE #papo +o emily")
        .event(|event| matches!(event, Event::ModeChanged { own_nick: Some(own), changes, .. } if own == "emily" && changes == &vec![ModeChange { mode: 'o', action: ModeAction::Add, arg: Some(ModeArg::__member(Casemapping::Rfc1459.nick("emily"), papo_core::Prefix::__new('o', '@'))) }]))
        .server(":alice!alice@staff.example MODE #papo +kl review-key-2026 50")
        .event(|event| {
            assert!(!format!("{event:?}").contains("review-key-2026"));
            matches!(event, Event::ModeChanged { changes, .. } if changes == &vec![
                ModeChange { mode: 'k', action: ModeAction::Add, arg: Some(ModeArg::Key(Secret::from("review-key-2026"))) },
                ModeChange { mode: 'l', action: ModeAction::Add, arg: Some(ModeArg::Value("50".into())) },
            ])
        })
        .server(":ergo.test MODE emily +i")
        .event(|event| matches!(event, Event::ModeChanged { subject: Subject::User(nick), actor: Source::Server { name, .. }, changes, .. } if nick == "emily" && name == "ERGO.TEST" && changes == &vec![ModeChange { mode: 'i', action: ModeAction::Add, arg: None }]))
        .no_event();
}

#[test]
fn mode_events_keep_user_server_and_bare_nick_sources() {
    Transcript::new(Config::default().nick("emily"))
        .server(":alice!~a@host.example MODE #papo +i")
        .event(|event| {
            matches!(
                event,
                Event::ModeChanged {
                    actor: Source::User {
                        nick,
                        username: Some(username),
                        host: Some(host),
                        ..
                    },
                    ..
                } if nick == "alice" && username == "~a" && host == "host.example"
            )
        })
        .server(":ergo.test MODE #papo -i")
        .event(|event| {
            matches!(
                event,
                Event::ModeChanged {
                    actor: Source::Server { name, .. },
                    ..
                } if name == "ERGO.TEST"
            )
        })
        .server(":ChanServ MODE #papo +m")
        .event(|event| {
            matches!(
                event,
                Event::ModeChanged {
                    actor: Source::User {
                        nick,
                        username: None,
                        host: None,
                        ..
                    },
                    ..
                } if nick == "ChanServ"
            )
        })
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
fn own_bot_mode_updates_the_permanent_user_row() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily BOT=B :are supported")
        .server(":ergo.test 221 emily +Bi")
        .inspect(|session| {
            assert_eq!(session.state().user("emily").unwrap().is_bot(), Some(true));
            assert!(session.state().user_modes().any(|mode| mode == 'B'));
        })
        .server(":emily MODE emily -B")
        .inspect(|session| {
            assert_eq!(session.state().user("emily").unwrap().is_bot(), Some(false));
            assert!(!session.state().user_modes().any(|mode| mode == 'B'));
        })
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
            assert_eq!(prefix_symbols(channel.member("dan").unwrap().prefixes()), ['@', '+']);
            assert_eq!(channel.member("dan").unwrap().rank(), Some(Rank::Op));
            assert!(channel.member("dan").unwrap().has(Rank::Voice));
            assert!(!channel.member("dan").unwrap().has(Rank::Halfop));
            assert_eq!(channel.member("erin").unwrap().rank(), None);
            assert_eq!(channel.member("frank").unwrap().rank(), Some(Rank::Op));
            assert!(!channel.member("frank").unwrap().has(Rank::Voice));
            assert_eq!(channel.mode('l'), None);
            assert_eq!(channel.mode('m'), Some(ModeValue::Set));
        })
        .ignore_rest();
}

#[test]
fn bounds_channel_modes_at_literal_sixty_four() {
    let modes = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij0lmnpqrstuwxyzαβγδεζηθικλμνξο";
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
        .server("CAP * LS :no-implicit-names")
        .client("CAP REQ :no-implicit-names")
        .server("CAP * ACK :no-implicit-names")
        .client("CAP END")
        .server(":ergo.test 005 papo WHOX :are supported")
        .server(":papo!papo@127.0.0.1 JOIN #papo")
        .client("NAMES #papo")
        .event(|event| matches!(event, Event::Joined { channel, nick, .. } if channel == "#papo" && nick == "papo"))
        .inspect(|session| assert!(!session.state().channel("#papo").unwrap().is_synced()))
        .server(":ergo.test 353 papo = #papo :@papo +emily")
        .server(":ergo.test 366 papo #papo :End of NAMES list")
        .client("MODE #papo")
        .client("WHO #papo %tcuhnfar,152")
        .event(|event| matches!(event, Event::ChannelSynced { channel, .. } if channel == "#papo"))
        .inspect(|session| assert!(session.state().channel("#papo").unwrap().is_synced()))
        .no_event();
}

#[test]
fn repeated_end_of_names_does_not_resync_a_channel() {
    Transcript::new(Config::default().who_on_join(WhoOnJoin::Never))
        .server(":papo!papo@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { channel, .. } if channel == "#papo"))
        .server(":ergo.test 366 papo #papo :End of NAMES list")
        .client("MODE #papo")
        .event(|event| matches!(event, Event::ChannelSynced { channel, .. } if channel == "#papo"))
        .server(":ergo.test 366 papo #papo :End of NAMES list")
        .server(":ergo.test 366 papo #papo :End of NAMES list")
        .server(":ergo.test 366 papo #papo :End of NAMES list")
        .no_client()
        .no_event();
}

#[test]
fn no_implicit_names_ignores_an_empty_implicit_end_before_the_requested_names() {
    Transcript::new(Config::default().wanted_caps(["no-implicit-names"]))
        .connect_default()
        .server("CAP * LS :no-implicit-names")
        .client("CAP REQ :no-implicit-names")
        .server("CAP * ACK :no-implicit-names")
        .client("CAP END")
        .server(":inspircd.test 005 papo WHOX :are supported")
        .server(":papo!papo@127.0.0.1 JOIN #papo")
        .client("NAMES #papo")
        .event(|event| matches!(event, Event::Joined { channel, .. } if channel == "#papo"))
        .server(":inspircd.test 366 papo #papo :End of /NAMES list")
        .inspect(|session| assert!(!session.state().channel("#papo").unwrap().is_synced()))
        .no_event()
        .server(":inspircd.test 353 papo = #papo :@papo +emily")
        .server(":inspircd.test 366 papo #papo :End of /NAMES list")
        .client("MODE #papo")
        .client("WHO #papo %tcuhnfar,152")
        .event(|event| matches!(event, Event::ChannelSynced { channel, .. } if channel == "#papo"))
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert!(channel.is_synced());
            assert!(channel.member("emily").unwrap().has(Rank::Voice));
        })
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
        .event(|event| matches!(event, Event::Joined { channel, nick, .. } if channel == "#Papo" && nick == "papo"))
        .inspect(|session| assert!(!session.state().channel("#papo").unwrap().is_synced()))
        .server(":ergo.test 366 papo #Papo :End of NAMES list")
        .client("MODE #Papo")
        .client("WHO #Papo")
        .event(|event| matches!(event, Event::ChannelSynced { channel, .. } if channel == "#Papo"))
        .server(":solanum.example 324 papo #Papo +ntk secret")
        .server(":solanum.example 329 papo #Papo 1760529342")
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert!(channel.is_synced());
            assert_eq!(channel.mode('n'), Some(ModeValue::Set));
            assert_eq!(channel.mode('k'), Some(ModeValue::Set));
            assert_eq!(channel.key().map(Secret::expose), Some("secret"));
            assert_eq!(channel.created_at(), Some(UNIX_EPOCH + Duration::from_secs(1_760_529_342)));
        })
        .no_event();
}

#[test]
fn exposes_channel_replies_for_unjoined_channels_as_messages() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 353 emily = #opers :@alice +dan")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("353")))
        .server(":ergo.test 366 emily #opers :End of NAMES list")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("366")))
        .server(":solanum.example 332 emily #staff :Operator coordination")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("332")))
        .server(":solanum.example 333 emily #staff alice 1760529342")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("333")))
        .server(":irc.unreal.test 324 emily #services +snt")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("324")))
        .server(":irc.unreal.test 329 emily #services 1760529342")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("329")))
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
            assert_eq!(session.state_generation(), 8);
        })
        .server(":dan!dan@cloak.example QUIT :Client exited")
        .inspect(|session| {
            assert_eq!(session.state_generation(), 9);
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
                limit: StateLimit::Channels,
                ..
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
                limit: StateLimit::Members,
                ..
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
        .inspect(|session| {
            assert_eq!(
                session.state().user("papo-bot").unwrap().is_bot(),
                Some(true)
            );
        })
        .ignore_rest();
}

#[test]
fn sweeps_joined_channels_with_a_rate_limited_whox_query() {
    Transcript::rate_limited(
        Config::default()
            .nick("emily")
            .rate_limit(RateLimit::new(1, Duration::from_millis(500)).unwrap()),
    )
    .connect()
    .client("CAP LS 302")
    .client("NICK emily")
    .client("USER papo 0 * papo")
    .send(Line::privmsg("#papo", "warmup"))
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
    .event(|event| matches!(event, Event::ChannelSynced { channel, .. } if channel == "#papo"))
    .server(":ergo.test 354 emily 152 #papo em 127.0.0.1 emily H@ emily :Emily Garcia")
    .server(":ergo.test 354 emily 152 #papo ~dan account/dan dan GB+ dan :Daniel Garcia")
    .server(":ergo.test 315 emily #papo :End of WHO list")
    .event(|event| matches!(event, Event::MembersRefreshed { channel, .. } if channel == "#papo"))
    .inspect(|session| {
        let dan = session.state().user("dan").unwrap();
        assert_eq!(
            dan.account(),
            &Account::LoggedIn(Casemapping::default().account("dan"))
        );
        assert_eq!(dan.username(), Some("~dan"));
        assert_eq!(dan.host(), Some("account/dan"));
        assert_eq!(dan.realname(), Some("Daniel Garcia"));
        assert_eq!(dan.away(), &Away::__away_value(None));
        assert_eq!(dan.is_bot(), Some(true));
    })
    .server(":alice!~alice@staff.example JOIN #papo alice :Alice Smith")
    .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "alice"))
    .no_client()
    .no_event();
}

#[test]
fn never_skips_the_join_sweep_and_leaves_member_facts_unknown() {
    Transcript::new(
        Config::default()
            .nick("emily")
            .who_on_join(WhoOnJoin::Never),
    )
    .server(":ergo.test 005 emily WHOX :are supported")
    .server(":emily!em@127.0.0.1 JOIN #papo")
    .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "emily"))
    .server(":ergo.test 353 emily = #papo :@emily +dan")
    .server(":ergo.test 366 emily #papo :End of NAMES list")
    .client("MODE #papo")
    .event(|event| matches!(event, Event::ChannelSynced { channel, .. } if channel == "#papo"))
    .no_client()
    .inspect(|session| {
        let dan = session.state().user("dan").unwrap();
        assert_eq!(dan.account(), &Account::Unknown);
        assert_eq!(dan.away(), &Away::Unknown);
    })
    .no_event();
}

#[test]
fn manual_sweep_refreshes_member_facts_and_completes_at_end_of_who() {
    Transcript::new(
        Config::default()
            .nick("emily")
            .who_on_join(WhoOnJoin::Never),
    )
    .server(":ergo.test 005 emily WHOX PREFIX=(ov)@+ :are supported")
    .server(":emily!em@127.0.0.1 JOIN #papo")
    .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "emily"))
    .server(":ergo.test 353 emily = #papo :@emily +dan")
    .server(":ergo.test 366 emily #papo :End of NAMES list")
    .client("MODE #papo")
    .event(|event| matches!(event, Event::ChannelSynced { channel, .. } if channel == "#papo"))
    .update(|session| {
        session
            .request_with_matcher(
                Instant::now(),
                Line::new("WHO").with_params(["#papo", "%tcuhnfar,152"]),
                ResponseMatcher::Who("#papo".into()),
            )
            .unwrap();
    })
    .client("WHO #papo %tcuhnfar,152")
    .server(":ergo.test 354 emily 152 #papo ~dan account/dan dan G+ dan :Daniel Garcia")
    .server(":ergo.test 315 emily #papo :End of WHO list")
    .event(|event| matches!(event, Event::MembersRefreshed { channel, .. } if channel == "#papo"))
    .event(|event| matches!(event, Event::Response(response) if response.lines.len() == 2))
    .inspect(|session| {
        let dan = session.state().user("dan").unwrap();
        assert_eq!(
            dan.account(),
            &Account::LoggedIn(Casemapping::default().account("dan"))
        );
        assert_eq!(dan.away(), &Away::__away_value(None));
    })
    .no_event();
}

#[test]
fn below_members_sweeps_only_below_the_literal_boundary() {
    Transcript::new(
        Config::default()
            .nick("emily")
            .who_on_join(WhoOnJoin::BelowMembers(2)),
    )
    .server(":ergo.test 005 emily WHOX :are supported")
    .server(":emily!em@127.0.0.1 JOIN #small")
    .event(|event| matches!(event, Event::Joined { channel, .. } if channel == "#small"))
    .server(":ergo.test 353 emily = #small :@emily")
    .server(":ergo.test 366 emily #small :End of NAMES list")
    .client("MODE #small")
    .client("WHO #small %tcuhnfar,152")
    .event(|event| matches!(event, Event::ChannelSynced { channel, .. } if channel == "#small"))
    .server(":ergo.test 315 emily #small :End of WHO list")
    .event(|event| matches!(event, Event::MembersRefreshed { channel, .. } if channel == "#small"))
    .server(":emily!em@127.0.0.1 JOIN #boundary")
    .event(|event| matches!(event, Event::Joined { channel, .. } if channel == "#boundary"))
    .server(":ergo.test 353 emily = #boundary :@emily +dan")
    .server(":ergo.test 366 emily #boundary :End of NAMES list")
    .client("MODE #boundary")
    .event(|event| matches!(event, Event::ChannelSynced { channel, .. } if channel == "#boundary"))
    .no_client()
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
        .event(|event| matches!(event, Event::ChannelSynced { channel, .. } if channel == "#papo"))
        .server(":ergo.test 352 emily #papo ~dan account/dan ergo.test dan GB+ :0 Daniel Garcia")
        .server(":ergo.test 315 emily #papo :End of WHO list")
        .event(
            |event| matches!(event, Event::MembersRefreshed { channel, .. } if channel == "#papo"),
        )
        .inspect(|session| {
            let dan = session.state().user("dan").unwrap();
            assert_eq!(dan.account(), &Account::Unknown);
            assert_eq!(dan.username(), Some("~dan"));
            assert_eq!(dan.host(), Some("account/dan"));
            assert_eq!(dan.realname(), Some("Daniel Garcia"));
            assert_eq!(dan.away(), &Away::__away_value(None));
            assert_eq!(dan.is_bot(), Some(true));
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
                    away: Away::Away {
                        message: Some(_),
                        ..
                    },
                    ..
                }
            )
        })
        .server(":ergo.test 366 emily #papo :End of NAMES list")
        .client("MODE #papo")
        .client("WHO #papo")
        .event(|event| matches!(event, Event::ChannelSynced { .. }))
        .server(":ergo.test 352 emily #papo ~dan cloak.example ergo.test dan H :0 Daniel Garcia")
        .inspect(|session| assert_eq!(session.state().user("dan").unwrap().away(), &Away::Present))
        .no_event();
}

#[test]
fn who_gone_flags_preserve_a_known_away_message() {
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
                    away: Away::Away {
                        message: Some(_),
                        ..
                    },
                    ..
                }
            )
        })
        .server(":ergo.test 366 emily #papo :End of NAMES list")
        .client("MODE #papo")
        .client("WHO #papo")
        .event(|event| matches!(event, Event::ChannelSynced { .. }))
        .server(":ergo.test 352 emily #papo ~dan cloak.example ergo.test dan G :0 Daniel Garcia")
        .inspect(|session| {
            assert_eq!(
                session.state().user("dan").unwrap().away(),
                &Away::__away_value(Some("Gone to lunch".into()))
            );
        })
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
            assert_eq!(session.state().user("service-bot").unwrap().is_bot(), Some(true));
            assert_eq!(session.state().user("relay-bot").unwrap().is_bot(), Some(true));
            assert_eq!(session.state().user("alice").unwrap().is_bot(), Some(false));
        })
        .no_event();
}

#[test]
fn bot_state_is_unknown_until_reported() {
    Transcript::new(Config::default().nick("emily"))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .server(":service-bot!bot@services.example JOIN #papo")
        .inspect(|session| {
            assert_eq!(session.state().user("service-bot").unwrap().is_bot(), None);
        })
        .server(":ergo.test 335 emily service-bot :is a bot on this network")
        .inspect(|session| {
            assert_eq!(
                session.state().user("service-bot").unwrap().is_bot(),
                Some(true)
            );
        })
        .ignore_rest();
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
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().raw_command() == "354"))
        .server(":ergo.test 315 emily #papo :End of WHO list")
        .event(|event| matches!(event, Event::MembersRefreshed { channel, .. } if channel == "#papo"))
        .server(":ergo.test 354 emily 152 #papo alice users.example alice H alice :Alice Smith")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().raw_command() == "354"))
        .inspect(|session| {
            assert!(session.state().user("dan").is_none());
            assert!(session.state().user("alice").is_none());
        })
        .no_event();
}

#[test]
fn user_requested_who_replies_remain_messages() {
    Transcript::new(Config::default().nick("emily"))
        .send(Line::new("WHO").with_params(["#rust"]))
        .client("WHO #rust")
        .server(":ergo.test 352 emily #rust ferris user/ferris ergo.test ferris H :0 Ferris Crab")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().raw_command() == "352"))
        .server(":ergo.test 315 emily #rust :End of WHO list")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().raw_command() == "315"))
        .no_event();
}

#[test]
fn reports_parts_and_kicks_for_other_users_and_ourselves() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily CASEMAPPING=ascii :are supported")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "emily"))
        .server(":dan!dan@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "dan"))
        .server(":dan!dan@cloak.example PART #papo :Leaving")
        .event(|event| matches!(event, Event::Parted { actor, channel, nick, reason, .. } if actor == "dan!dan@cloak.example" && channel == "#papo" && nick == "dan" && channel.casemapping() == Casemapping::Ascii && nick.casemapping() == Casemapping::Ascii && reason.as_deref() == Some("Leaving")))
        .server(":dan!dan@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "dan"))
        .server(":chanop!op@staff.example KICK #papo dan :Rule violation")
        .event(|event| matches!(event, Event::Kicked { channel, subject, actor, reason, .. } if channel == "#papo" && subject == "dan" && channel.casemapping() == Casemapping::Ascii && subject.casemapping() == Casemapping::Ascii && actor.nick().is_some_and(|nick| nick == "chanop") && reason.as_deref() == Some("Rule violation")))
        .server(":emily!em@127.0.0.1 PART #papo :Requested")
        .event(|event| matches!(event, Event::Parted { nick, .. } if nick == "emily"))
        .server(":emily!em@127.0.0.1 JOIN #rust")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "emily"))
        .server(":chanop!op@staff.example KICK #rust emily :Requested")
        .event(|event| matches!(event, Event::Kicked { subject, .. } if subject == "emily"))
        .no_event();
}

#[test]
fn kick_events_keep_user_server_and_bare_nick_sources() {
    Transcript::new(Config::default().nick("emily"))
        .server(":alice!~a@host.example KICK #papo bob :Flooding")
        .event(|event| {
            matches!(
                event,
                Event::Kicked {
                    actor: Source::User {
                        nick,
                        username: Some(username),
                        host: Some(host),
                        ..
                    },
                    ..
                } if nick == "alice" && username == "~a" && host == "host.example"
            )
        })
        .server(":ergo.test KICK #papo bob :Channel cleanup")
        .event(|event| {
            matches!(
                event,
                Event::Kicked {
                    actor: Source::Server { name, .. },
                    ..
                } if name == "ERGO.TEST"
            )
        })
        .server(":ChanServ KICK #papo bob :Requested")
        .event(|event| {
            matches!(
                event,
                Event::Kicked {
                    actor: Source::User {
                        nick,
                        username: None,
                        host: None,
                        ..
                    },
                    ..
                } if nick == "ChanServ"
            )
        })
        .no_event();
}

#[test]
fn empty_departure_reasons_are_absent() {
    Transcript::new(Config::default().nick("papo"))
        .server(":papo!papo@localhost JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":alice!~alice@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":alice!~alice@cloak.example PART #papo :")
        .event(|event| matches!(event, Event::Parted { reason: None, .. }))
        .server(":alice!~alice@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":chanop!op@staff.example KICK #papo alice :")
        .event(|event| matches!(event, Event::Kicked { reason: None, .. }))
        .server(":alice!~alice@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":alice!~alice@cloak.example QUIT :")
        .event(|event| matches!(event, Event::Quit { reason: None, .. }))
        .no_event();
}

#[test]
fn reports_nick_changes_and_quit_channels_before_removal() {
    Transcript::new(Config::default().nick("emily"))
        .server(":ergo.test 005 emily CASEMAPPING=ascii :are supported")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .server(":emily!em@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":emily!em@127.0.0.1 JOIN #rust")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":dan!dan@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":dan!dan@cloak.example JOIN #rust")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":dan!dan@cloak.example NICK :dan-away")
        .event(|event| matches!(event, Event::NickChanged { actor, old, new, channels, .. } if actor == "dan!dan@cloak.example" && old == "dan" && new == "dan-away" && *channels == ["#papo", "#rust"] && old.casemapping() == Casemapping::Ascii && new.casemapping() == Casemapping::Ascii))
        .server(":emily!em@127.0.0.1 NICK :emily_")
        .event(|event| matches!(event, Event::NickChanged { old, new, channels, .. } if old == "emily" && new == "emily_" && *channels == ["#papo", "#rust"]))
        .server(":dan-away!dan@cloak.example QUIT :Client exited")
        .event(|event| match event {
            Event::Quit { actor, nick, reason, channels, .. } => {
                let mut channels = channels.clone();
                channels.sort();
                actor == "dan-away!dan@cloak.example"
                    && nick == "dan-away"
                    && reason.as_deref() == Some("Client exited")
                    && nick.casemapping() == Casemapping::Ascii
                    && channels
                        .iter()
                        .all(|channel| channel.casemapping() == Casemapping::Ascii)
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
        .server(":emily!em@127.0.0.1 JOIN #Papo[dev] emily :Emily Example")
        .server(":Alice[bot]!alice@users.example JOIN #Papo[dev] Alice[bot] :Alice Bot")
        .inspect(|session| {
            let channel = session.state().channel("#papo[dev]").unwrap();
            let user = session.state().user("alice[bot]").unwrap();
            assert_eq!(session.state().nick().casemapping(), Casemapping::Ascii);
            assert_eq!(channel.name().casemapping(), Casemapping::Ascii);
            assert_eq!(channel.member("alice[bot]").unwrap().nick().casemapping(), Casemapping::Ascii);
            assert_eq!(user.nick().casemapping(), Casemapping::Ascii);
            assert!(matches!(user.account(), Account::LoggedIn(account) if account.casemapping() == Casemapping::Ascii));
            assert!(session.state().channel("#papo{dev}").is_none());
            assert!(session.state().user("alice{bot}").is_none());
        })
        .server(":ergo.test 005 emily CASEMAPPING=rfc1459 :are supported")
        .inspect(|session| {
            let channel = session.state().channel("#papo{dev}").unwrap();
            let member = channel.member("alice{bot}").unwrap();
            let user = session.state().user("alice{bot}").unwrap();
            assert_eq!(session.state().nick().casemapping(), Casemapping::Rfc1459);
            assert_eq!(channel.name().casemapping(), Casemapping::Rfc1459);
            assert_eq!(member.nick().casemapping(), Casemapping::Rfc1459);
            assert_eq!(user.nick().casemapping(), Casemapping::Rfc1459);
            assert!(matches!(user.account(), Account::LoggedIn(account) if account.casemapping() == Casemapping::Rfc1459));
        })
        .ignore_rest();
}
