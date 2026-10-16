//! Transcript tests for `IRCv3` MONITOR state.

mod support;

use std::cell::Cell;

use papo_core::{Account, Config, Event};
use papo_proto::Message;
use support::Transcript;

fn registered() -> Transcript {
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 005 papo MONITOR=100 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, papo_core::Event::Registered(_)))
}

fn extended_registered() -> Transcript {
    Transcript::new(Config::default().wanted_caps(["extended-monitor"]))
        .connect_default()
        .server("CAP * LS :extended-monitor")
        .client("CAP REQ :extended-monitor")
        .server("CAP * ACK :extended-monitor")
        .client("CAP END")
        .server(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 005 papo MONITOR=100 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Registered(_)))
}

#[test]
fn adds_removes_and_clears_monitored_nicks() {
    registered()
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .client("MONITOR + alice,bob,carol")
        .update(|session| session.monitor_remove(["BOB"]))
        .client("MONITOR - bob")
        .update(papo_core::Session::monitor_clear)
        .client("MONITOR C")
        .no_client();
}

#[test]
fn monitor_updates_generation_only_when_presence_changes() {
    let generation = Cell::new(0);
    extended_registered()
        .update(|session| session.monitor_add(["alice", "bob"]))
        .client("MONITOR + alice,bob")
        .server(":ergo.test 730 papo :alice!~alice@cloak.example,bob!bob@users.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "bob"))
        .inspect(|session| generation.set(session.state_generation()))
        .update(|session| session.monitor_remove(["zoe"]))
        .inspect(|session| assert_eq!(session.state_generation(), generation.get()))
        .no_client()
        .update(|session| session.monitor_remove(["alice"]))
        .client("MONITOR - alice")
        .inspect(|session| {
            assert_eq!(session.state_generation(), generation.get() + 1);
            generation.set(session.state_generation());
        })
        .update(papo_core::Session::monitor_clear)
        .client("MONITOR C")
        .inspect(|session| {
            assert_eq!(session.state_generation(), generation.get() + 1);
            generation.set(session.state_generation());
        })
        .update(papo_core::Session::monitor_clear)
        .client("MONITOR C")
        .inspect(|session| assert_eq!(session.state_generation(), generation.get()))
        .no_event();
}

#[test]
fn removing_a_monitored_nick_fills_the_advertised_slot() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 005 papo MONITOR=2 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Registered(_)))
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .client("MONITOR + alice,bob")
        .event(|event| matches!(event, Event::MonitorListFull { limit: 2, nicks } if nicks == &["carol"]))
        .update(|session| session.monitor_remove(["alice"]))
        .client("MONITOR - alice")
        .client("MONITOR + carol")
        .update(|session| session.monitor_remove(["zoe"]))
        .no_client();
}

#[test]
fn removal_distinguishes_unknown_active_and_unsent_nicks() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 005 papo MONITOR=1 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Registered(_)))
        .update(|session| session.monitor_add(["alice", "bob"]))
        .client("MONITOR + alice")
        .event(|event| matches!(event, Event::MonitorListFull { limit: 1, nicks } if nicks == &["bob"]))
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .update(|session| session.monitor_remove(["zoe"]))
        .no_client()
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .no_event()
        .update(|session| session.monitor_remove(["bob"]))
        .no_client()
        .update(|session| session.monitor_remove(["alice"]))
        .client("MONITOR - alice")
        .update(|session| session.monitor_add(["alice"]))
        .client("MONITOR + alice")
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .no_event();
}

#[test]
fn removing_before_registration_or_without_support_sends_nothing() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test 005 * MONITOR=100 :are supported")
        .update(|session| session.monitor_add(["alice"]))
        .update(|session| session.monitor_remove(["alice"]))
        .no_client();

    Transcript::new(Config::default())
        .connect_default()
        .server(":solanum.example 001 papo :Welcome")
        .server(":solanum.example 422 papo :MOTD File is missing")
        .event(|event| matches!(event, Event::Registered(_)))
        .update(|session| session.monitor_add(["alice"]))
        .event(|event| matches!(event, Event::MonitorUnsupported))
        .update(|session| session.monitor_remove(["alice"]))
        .no_client();
}

#[test]
fn honors_the_advertised_monitor_limit_before_chunking() {
    let nicks: Vec<_> = (0..150).map(|index| format!("user{index:03}")).collect();
    let unavailable = nicks[100..].to_vec();
    let lines = [0..62, 62..100].map(|range| format!("MONITOR + {}", nicks[range].join(",")));
    registered()
        .update(|session| session.monitor_add(nicks))
        .client(&lines[0])
        .client(&lines[1])
        .no_client()
        .event(|event| matches!(event, Event::MonitorListFull { limit: 100, nicks } if nicks == &unavailable));
}

#[test]
fn chunks_monitor_targets_at_the_literal_512_byte_boundary() {
    let first = "a".repeat(249);
    let second = "b".repeat(250);
    let third = "c".repeat(32);
    let exact = format!("MONITOR + {first},{second}");
    assert_eq!(exact.len() + 2, 512);
    registered()
        .update(|session| session.monitor_add([first, second, third.clone()]))
        .client(&exact)
        .client(&format!("MONITOR + {third}"))
        .no_client();

    let first = "a".repeat(249);
    let second = "b".repeat(251);
    let over = format!("MONITOR + {first},{second}");
    assert_eq!(over.len() + 2, 513);
    registered()
        .update(|session| session.monitor_add([first.clone(), second.clone()]))
        .client(&format!("MONITOR + {first}"))
        .client(&format!("MONITOR + {second}"))
        .no_client();
}

#[test]
fn tracks_online_offline_and_list_full_replies() {
    registered()
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .client("MONITOR + alice,bob,carol")
        .server(":ergo.test 730 papo :alice!~alice@cloak.example,bob,carol!carol@staff.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, user, host } if nick == "alice" && user.as_deref() == Some("~alice") && host.as_deref() == Some("cloak.example")))
        .event(|event| matches!(event, Event::MonitorOnline { nick, user, host } if nick == "bob" && user.is_none() && host.is_none()))
        .event(|event| matches!(event, Event::MonitorOnline { nick, user, host } if nick == "carol" && user.as_deref() == Some("carol") && host.as_deref() == Some("staff.example")))
        .inspect(|session| {
            let mut online: Vec<_> = session
                .state()
                .monitor_online()
                .map(|user| (user.nick(), user.user(), user.host()))
                .collect();
            online.sort_unstable();
            assert_eq!(
                online,
                [
                    ("alice", Some("~alice"), Some("cloak.example")),
                    ("bob", None, None),
                    ("carol", Some("carol"), Some("staff.example")),
                ]
            );
        })
        .server(":ergo.test 730 papo :bob")
        .no_event()
        .server(":ergo.test 730 papo :bob!bob@users.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, user, host } if nick == "bob" && user.as_deref() == Some("bob") && host.as_deref() == Some("users.example")))
        .server(":ergo.test 730 papo :ALICE!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "ALICE"))
        .server(":ergo.test 731 papo :alice")
        .event(|event| matches!(event, Event::MonitorOffline { nick } if nick == "alice"))
        .server(":ergo.test 731 papo :alice")
        .no_event()
        .server(":ergo.test 734 papo 100 carol :Monitor list is full")
        .event(|event| matches!(event, Event::MonitorListFull { limit: 100, nicks } if nicks == &["carol"]))
        .no_event();
}

#[test]
fn list_full_forgets_only_the_rejected_presence() {
    registered()
        .update(|session| session.monitor_add(["alice", "bob"]))
        .client("MONITOR + alice,bob")
        .server(":ergo.test 730 papo :alice!~alice@cloak.example,bob!bob@users.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "bob"))
        .server(":ergo.test 734 papo 100 alice :Monitor list is full")
        .event(|event| matches!(event, Event::MonitorListFull { nicks, .. } if nicks == &["alice"]))
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .server(":ergo.test 730 papo :bob!bob@users.example")
        .no_event();
}

#[test]
fn malformed_monitor_replies_remain_messages() {
    registered()
        .server(":ergo.test 730 papo :!alice@cloak.example")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().raw_command() == "730"))
        .server(":ergo.test 730 papo :alice!@cloak.example")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().raw_command() == "730"))
        .server(":ergo.test 730 papo :alice!~alice@")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().raw_command() == "730"))
        .server(":ergo.test 731 papo :")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().raw_command() == "731"))
        .server(":ergo.test 734 papo many alice :Monitor list is full")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().raw_command() == "734"))
        .server(":ergo.test 734 papo 100 :")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().raw_command() == "734"))
        .no_event();
}

#[test]
fn restores_the_desired_list_and_resets_presence_after_reconnect() {
    registered()
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .client("MONITOR + alice,bob,carol")
        .server(":ergo.test 731 papo :alice")
        .event(|event| matches!(event, Event::MonitorOffline { nick } if nick == "alice"))
        .disconnect()
        .connect_default()
        .server(":ergo.test 001 papo :Welcome back to Ergo")
        .server(":ergo.test 005 papo MONITOR=100 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Registered(_)))
        .client("MONITOR + alice,bob,carol")
        .server(":ergo.test 731 papo :alice")
        .event(|event| matches!(event, Event::MonitorOffline { nick } if nick == "alice"))
        .no_event();
}

#[test]
fn disconnect_resets_a_confirmed_online_state() {
    registered()
        .update(|session| session.monitor_add(["alice"]))
        .client("MONITOR + alice")
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .disconnect()
        .connect_default()
        .server(":ergo.test 001 papo :Welcome back")
        .server(":ergo.test 005 papo MONITOR=100 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Registered(_)))
        .client("MONITOR + alice")
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .no_event();
}

#[test]
fn restore_distinguishes_the_literal_monitor_limit_boundary() {
    let register = |nicks| {
        Transcript::new(Config::default())
            .connect_default()
            .server(":ergo.test 001 papo :Welcome")
            .server(":ergo.test 005 papo MONITOR=2 :are supported")
            .server(":ergo.test 376 papo :End of MOTD")
            .event(|event| matches!(event, Event::Registered(_)))
            .update(|session| session.monitor_add(nicks))
    };

    register(vec!["alice", "bob"])
        .client("MONITOR + alice,bob")
        .disconnect()
        .connect_default()
        .server(":ergo.test 001 papo :Welcome back")
        .server(":ergo.test 005 papo MONITOR=2 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Registered(_)))
        .client("MONITOR + alice,bob")
        .no_event();

    register(vec!["alice", "bob", "carol"])
        .client("MONITOR + alice,bob")
        .event(|event| matches!(event, Event::MonitorListFull { limit: 2, nicks } if nicks == &["carol"]))
        .disconnect()
        .connect_default()
        .server(":ergo.test 001 papo :Welcome back")
        .server(":ergo.test 005 papo MONITOR=2 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Registered(_)))
        .client("MONITOR + alice,bob")
        .event(|event| matches!(event, Event::MonitorListFull { limit: 2, nicks } if nicks == &["carol"]))
        .no_event();
}

#[test]
fn extended_monitor_updates_an_unshared_users_record() {
    extended_registered()
        .update(|session| session.monitor_add(["alice"]))
        .client("MONITOR + alice")
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .server(":alice!~alice@cloak.example AWAY :Gone to lunch")
        .event(|event| matches!(event, Event::AwayChanged { nick, message } if nick == "alice" && message.as_deref() == Some("Gone to lunch")))
        .server(":alice!~alice@cloak.example ACCOUNT alice")
        .event(|event| matches!(event, Event::AccountChanged { nick, account } if nick == "alice" && account == &Account::LoggedIn("alice".into())))
        .server(":alice!~alice@cloak.example CHGHOST alice account/alice")
        .event(|event| matches!(event, Event::HostChanged { nick, .. } if nick == "alice"))
        .server(":alice!alice@account/alice SETNAME :Alice Smith")
        .event(|event| matches!(event, Event::RealnameChanged { nick, .. } if nick == "alice"))
        .inspect(|session| {
            let alice = session.state().user("alice").unwrap();
            assert_eq!(alice.channels().count(), 0);
            assert_eq!(alice.away(), Some("Gone to lunch"));
            assert_eq!(alice.account(), &Account::LoggedIn("alice".into()));
            assert_eq!(alice.host(), Some("account/alice"));
            assert_eq!(alice.realname(), Some("Alice Smith"));
        })
        .server(":ergo.test 731 papo :alice")
        .event(|event| matches!(event, Event::MonitorOffline { nick } if nick == "alice"))
        .inspect(|session| assert!(session.state().user("alice").is_none()))
        .no_event();
}

#[test]
fn member_departures_retain_monitored_or_still_shared_users() {
    extended_registered()
        .update(|session| session.monitor_add(["alice"]))
        .client("MONITOR + alice")
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .server(":papo!papo@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "papo"))
        .server(":alice!~alice@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "alice"))
        .server(":bob!bob@users.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "bob"))
        .server(":papo!papo@127.0.0.1 JOIN #rust")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "papo"))
        .server(":bob!bob@users.example JOIN #rust")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "bob"))
        .server(":alice!~alice@cloak.example PART #papo :Leaving")
        .event(|event| matches!(event, Event::Parted { nick, .. } if nick == "alice"))
        .server(":bob!bob@users.example PART #papo :Leaving")
        .event(|event| matches!(event, Event::Parted { nick, .. } if nick == "bob"))
        .inspect(|session| {
            assert_eq!(session.state().user("alice").unwrap().channels().count(), 0);
            assert_eq!(session.state().user("bob").unwrap().channels().count(), 1);
        })
        .server(":bob!bob@users.example PART #rust :Leaving")
        .event(|event| matches!(event, Event::Parted { nick, .. } if nick == "bob"))
        .inspect(|session| assert!(session.state().user("bob").is_none()))
        .no_event();
}

#[test]
fn leaving_a_channel_retains_only_monitored_or_still_shared_users() {
    extended_registered()
        .update(|session| session.monitor_add(["alice"]))
        .client("MONITOR + alice")
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .server(":papo!papo@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "papo"))
        .server(":alice!~alice@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "alice"))
        .server(":bob!bob@users.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "bob"))
        .server(":dan!dan@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "dan"))
        .server(":papo!papo@127.0.0.1 JOIN #rust")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "papo"))
        .server(":dan!dan@cloak.example JOIN #rust")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "dan"))
        .server(":papo!papo@127.0.0.1 PART #papo :Leaving")
        .event(|event| matches!(event, Event::Parted { nick, .. } if nick == "papo"))
        .inspect(|session| {
            assert_eq!(session.state().user("alice").unwrap().channels().count(), 0);
            assert!(session.state().user("bob").is_none());
            assert_eq!(session.state().user("dan").unwrap().channels().count(), 1);
        })
        .no_event();
}

#[test]
fn timeout_restores_only_when_registration_becomes_complete() {
    Transcript::new(Config::default())
        .connect_default()
        .update(|session| session.monitor_add(["alice"]))
        .server(":soju.example 001 papo :Welcome")
        .server(":soju.example 005 papo MONITOR=100 :are supported")
        .advance(std::time::Duration::from_secs(9))
        .no_client()
        .advance(std::time::Duration::from_secs(1))
        .event(|event| matches!(event, Event::Registered(_)))
        .client("MONITOR + alice")
        .no_client();
}

#[test]
fn monitor_clear_requires_registration_and_server_support() {
    registered()
        .update(papo_core::Session::monitor_clear)
        .client("MONITOR C")
        .no_client();

    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test 005 * MONITOR=100 :are supported")
        .update(|session| session.monitor_add(["alice"]))
        .update(papo_core::Session::monitor_clear)
        .no_client();

    Transcript::new(Config::default())
        .connect_default()
        .server(":solanum.example 001 papo :Welcome")
        .server(":solanum.example 422 papo :MOTD File is missing")
        .event(|event| matches!(event, Event::Registered(_)))
        .update(papo_core::Session::monitor_clear)
        .no_client()
        .update(|session| session.monitor_add(["alice"]))
        .event(|event| matches!(event, Event::MonitorUnsupported))
        .update(papo_core::Session::monitor_clear)
        .no_client();
}

#[test]
fn plain_monitor_presence_is_removed_after_offline() {
    registered()
        .update(|session| session.monitor_add(["alice"]))
        .client("MONITOR + alice")
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .inspect(|session| assert_eq!(session.state().monitor_online().count(), 1))
        .server(":ergo.test 731 papo :alice")
        .event(|event| matches!(event, Event::MonitorOffline { nick } if nick == "alice"))
        .inspect(|session| {
            assert_eq!(session.state().monitor_online().count(), 0);
            assert!(session.state().user("alice").is_none());
        })
        .no_event();
}

#[test]
fn reports_unsupported_monitor_only_once() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":solanum.example 001 papo :Welcome")
        .server(":solanum.example 422 papo :MOTD File is missing")
        .event(|event| matches!(event, Event::Registered(_)))
        .update(|session| session.monitor_add(["alice"]))
        .event(|event| matches!(event, Event::MonitorUnsupported))
        .no_client()
        .update(|session| session.monitor_add(["bob"]))
        .no_event()
        .no_client();
}

#[test]
fn caps_the_desired_monitor_list_at_ten_thousand() {
    let nicks = (0..10_001).map(|index| format!("user{index:05}"));
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test 001 papo :Welcome")
        .server(":ergo.test 005 papo MONITOR :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Registered(_)))
        .update(|session| session.monitor_add(nicks))
        .event(|event| matches!(event, Event::MonitorListFull { limit: 10_000, nicks } if nicks == &["user10000"]))
        .ignore_rest();
}

#[test]
fn user_requested_monitor_list_replies_remain_messages() {
    registered()
        .send(Message::new("MONITOR", ["L"]))
        .client("MONITOR L")
        .server(":ergo.test 732 papo :alice,bob")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().raw_command() == "732"))
        .server(":ergo.test 733 papo :End of MONITOR list")
        .event(|event| matches!(event, Event::Message(message) if message.as_ref().raw_command() == "733"))
        .no_event();
}
