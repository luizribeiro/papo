//! Transcript tests for `IRCv3` MONITOR state.

mod support;

use std::{
    cell::{Cell, RefCell},
    time::Instant,
};

use papo_core::{Account, Away, Config, Event, Feature, Session, UnsupportedReason};
use papo_proto::{Casemapping, Line, Nick, Origin};
use support::Transcript;

fn registered() -> Transcript {
    Transcript::new(Config::default())
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 005 papo MONITOR=100 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, papo_core::Event::Connected(_)))
}

#[test]
fn monitor_events_keep_the_session_origin() {
    let origin = Origin::__new(73);
    let mut config = Config::default();
    config.origin = origin;
    Transcript::new(config)
        .update(|session| session.monitor_add(["alice"]))
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .event(|event| {
            matches!(event, Event::MonitorOnline { nick, .. }
            if nick.origin() == origin)
        });
}

fn extended_registered() -> Transcript {
    Transcript::new(Config::default().wanted_caps(["extended-monitor"]))
        .connect_default()
        .server("CAP * LS :extended-monitor")
        .client("CAP REQ :extended-monitor")
        .server("CAP * ACK :extended-monitor")
        .client("CAP END")
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 005 papo MONITOR=100 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
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
fn watch_never_removes_an_existing_app_entry() {
    registered()
        .update(|session| session.monitor_add(["alice"]))
        .client("MONITOR + alice")
        .update(|session| session.__reconcile_watches(vec!["Alice".into()]))
        .no_client()
        .update(papo_core::Session::monitor_clear)
        .no_client()
        .update(|session| session.__reconcile_watches(Vec::new()))
        .client("MONITOR - Alice")
        .no_client();
}

#[test]
fn guarded_direct_monitor_follows_rename_and_releases_its_watch() {
    let recipient = RefCell::new(None);
    registered()
        .update(|session| {
            let account =
                Account::LoggedIn(session.state().casemapping().account("alice-account"));
            let acquired = session.__direct_acquire(
                "alice",
                Some("~alice"),
                Some("cloak.example"),
                Account::Unknown,
            );
            *recipient.borrow_mut() = session.__direct_prove(&acquired, account);
        })
        .client("MONITOR + alice")
        .inspect(|session| {
            assert_eq!(
                session
                    .state()
                    .monitored()
                    .map(Nick::as_str)
                    .collect::<Vec<_>>(),
                ["alice"]
            );
            assert!(matches!(
                session.state().user("alice").map(papo_core::User::account),
                Some(Account::LoggedIn(account)) if account == "alice-account"
            ));
        })
        .server(":alice!~alice@cloak.example QUIT :Connection reset by peer")
        .event(|event| matches!(event, Event::Quit { nick, .. } if nick == "alice"))
        .update(|session| {
            let account =
                Account::LoggedIn(session.state().casemapping().account("alice-account"));
            let acquired = session.__direct_acquire("alice", None, None, Account::Unknown);
            *recipient.borrow_mut() = session.__direct_prove(&acquired, account);
            session.__direct_release(&acquired);
        })
        .update(|session| session.monitor_add(["bob"]))
        .client("MONITOR + bob")
        .inspect(|session| assert!(session.state().user("alice").is_some()))
        .update(|session| session.monitor_remove(["bob"]))
        .client("MONITOR - bob")
        .server(":alice!~alice@cloak.example NICK alicia")
        .client("MONITOR - alice")
        .client("MONITOR + alicia")
        .event(|event| matches!(event, Event::NickChanged { old, new, .. } if old == "alice" && new == "alicia"))
        .inspect(|session| assert!(session.state().user("alicia").is_some()))
        .update(|session| {
            session.__direct_release(recipient.borrow().as_ref().expect("recipient was acquired"));
        })
        .client("MONITOR - alicia")
        .inspect(|session| assert_eq!(session.state().monitored().count(), 0))
        .no_client();
}

#[test]
fn direct_acquisition_waits_for_server_identity_proof() {
    let now = std::time::Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    session.handle_line(
        now,
        b":offline.test 005 papo CASEMAPPING=rfc1459 MONITOR=100 :are supported",
    );
    let claimed = Account::LoggedIn(session.state().casemapping().account("alice-account"));
    let acquired = session.__direct_acquire("alice", None, None, claimed.clone());
    assert_eq!(acquired.__account(), &Account::Unknown);
    session.handle_line(
        now,
        b"@account=mallory-account :alice!~mallory@users.example PRIVMSG papo :review update",
    );
    assert!(matches!(
        session.state().user("alice").map(papo_core::User::account),
        Some(Account::LoggedIn(account)) if account == "mallory-account"
    ));
    assert_eq!(session.__direct_prove(&acquired, claimed), None);
    session.__direct_acquire("alice", None, None, Account::Unknown);
    assert!(matches!(
        session.state().user("alice").map(papo_core::User::account),
        Some(Account::LoggedIn(account)) if account == "mallory-account"
    ));
}

#[test]
fn invalid_nicks_are_never_sent_or_stored() {
    registered()
        .update(|session| session.monitor_add(["bad\rnick", "alice", "bad,nick"]))
        .client("MONITOR + alice")
        .inspect(|session| {
            assert_eq!(
                session
                    .state()
                    .monitored()
                    .map(Nick::as_str)
                    .collect::<Vec<_>>(),
                ["alice"]
            );
        })
        .disconnect()
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome back")
        .server(":ergo.test 005 papo MONITOR=100 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .client("MONITOR + alice")
        .no_client();
}

#[test]
fn unsolicited_presence_is_ignored_for_undesired_nicks() {
    registered()
        .update(|session| session.monitor_add(["alice"]))
        .client("MONITOR + alice")
        .server(":ergo.test 730 papo :mallory!~mallory@users.example")
        .server(":ergo.test 731 papo :mallory")
        .no_event()
        .inspect(|session| assert_eq!(session.state().monitor_online().count(), 0))
        .server(":ergo.test 730 papo :alice!~alice@cloak.example,mallory!~mallory@users.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .inspect(|session| {
            assert_eq!(
                session
                    .state()
                    .monitor_online()
                    .map(|user| user.nick().as_str())
                    .collect::<Vec<_>>(),
                ["alice"]
            );
        })
        .update(|session| session.monitor_remove(["alice"]))
        .client("MONITOR - alice")
        .server(":ergo.test 731 papo :alice")
        .no_event();
}

#[test]
fn desired_nicks_keep_insertion_order_after_add_and_remove() {
    Transcript::new(Config::default())
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 005 papo MONITOR=2 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .client("MONITOR + alice,bob")
        .event(|event| matches!(event, Event::MonitorListFull { nicks, .. } if nicks == &["carol"]))
        .update(|session| session.monitor_remove(["BOB"]))
        .client("MONITOR - bob")
        .client("MONITOR + carol")
        .update(|session| session.monitor_add(["dave"]))
        .event(|event| matches!(event, Event::MonitorListFull { nicks, .. } if nicks == &["dave"]))
        .inspect(|session| {
            assert_eq!(
                session
                    .state()
                    .monitored()
                    .map(Nick::as_str)
                    .collect::<Vec<_>>(),
                ["alice", "carol"]
            );
        })
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
fn adding_a_desired_nick_updates_the_state_generation_once() {
    registered()
        .inspect(|session| assert_eq!(session.state_generation(), 2))
        .update(|session| session.monitor_add(["alice"]))
        .client("MONITOR + alice")
        .inspect(|session| assert_eq!(session.state_generation(), 3))
        .update(|session| session.monitor_add(["ALICE"]))
        .inspect(|session| assert_eq!(session.state_generation(), 3))
        .no_client();
}

#[test]
fn removing_a_monitored_nick_fills_the_advertised_slot() {
    Transcript::new(Config::default())
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 005 papo MONITOR=2 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .client("MONITOR + alice,bob")
        .event(|event| matches!(event, Event::MonitorListFull { limit: 2, nicks, .. } if nicks == &["carol"]))
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
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 005 papo MONITOR=1 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .update(|session| session.monitor_add(["alice", "bob"]))
        .client("MONITOR + alice")
        .event(|event| matches!(event, Event::MonitorListFull { limit: 1, nicks, .. } if nicks == &["bob"]))
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
        .welcome(":solanum.example 001 papo :Welcome")
        .server(":solanum.example 422 papo :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
        .update(|session| session.monitor_add(["alice"]))
        .no_event()
        .update(|session| session.monitor_remove(["alice"]))
        .no_client();
}

#[test]
fn registration_clears_unconfirmed_monitor_presence() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test 005 * MONITOR=100 :are supported")
        .update(|session| session.monitor_add(["alice"]))
        .server(":ergo.test 730 * :alice!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .inspect(|session| {
            assert_eq!(session.state().monitor_online().count(), 1);
            assert_eq!(session.state().__monitor_presence("alice"), Some(true));
        })
        .welcome(":ergo.test 001 papo :Welcome")
        .server(":ergo.test 376 papo :End of MOTD")
        .client("MONITOR + alice")
        .event(|event| matches!(event, Event::Connected(_)))
        .inspect(|session| assert_eq!(session.state().monitor_online().count(), 0))
        .no_event();
}

#[test]
fn honors_the_advertised_monitor_limit_before_chunking() {
    let nicks: Vec<_> = (0..150).map(|index| format!("user{index:03}")).collect();
    let unavailable = nicks[100..]
        .iter()
        .map(|nick| Casemapping::default().nick(nick))
        .collect::<Vec<_>>();
    let lines = [0..62, 62..100].map(|range| format!("MONITOR + {}", nicks[range].join(",")));
    registered()
        .update(|session| session.monitor_add(nicks))
        .client(&lines[0])
        .client(&lines[1])
        .no_client()
        .event(|event| matches!(event, Event::MonitorListFull { limit: 100, nicks, .. } if nicks == &unavailable));
}

#[test]
fn chunks_monitor_targets_at_the_literal_512_byte_boundary() {
    let first = "a".repeat(249);
    let second = "b".repeat(250);
    let third = "c".repeat(32);
    let exact = format!("MONITOR + {first},{second}");
    assert_eq!(exact.len() + 2, 512);
    registered()
        .server(":ergo.test 005 papo NICKLEN=251 :are supported")
        .update(|session| session.monitor_add([first, second, third.clone()]))
        .client(&exact)
        .client(&format!("MONITOR + {third}"))
        .no_client();

    let first = "a".repeat(249);
    let second = "b".repeat(251);
    let over = format!("MONITOR + {first},{second}");
    assert_eq!(over.len() + 2, 513);
    registered()
        .server(":ergo.test 005 papo NICKLEN=251 :are supported")
        .update(|session| session.monitor_add([first.clone(), second.clone()]))
        .client(&format!("MONITOR + {first}"))
        .client(&format!("MONITOR + {second}"))
        .no_client();
}

#[test]
fn tracks_online_offline_and_list_full_replies() {
    registered()
        .server(":ergo.test 005 papo CASEMAPPING=ascii :are supported")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .client("MONITOR + alice,bob,carol")
        .server(":ergo.test 730 papo :alice!~alice@cloak.example,bob,carol!carol@staff.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, username, host, .. } if nick == "alice" && nick.casemapping() == Casemapping::Ascii && username.as_deref() == Some("~alice") && host.as_deref() == Some("cloak.example")))
        .event(|event| matches!(event, Event::MonitorOnline { nick, username, host, .. } if nick == "bob" && username.is_none() && host.is_none()))
        .event(|event| matches!(event, Event::MonitorOnline { nick, username, host, .. } if nick == "carol" && username.as_deref() == Some("carol") && host.as_deref() == Some("staff.example")))
        .inspect(|session| {
            let mut online: Vec<_> = session
                .state()
                .monitor_online()
                .map(|user| (user.nick().as_str(), user.username(), user.host()))
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
        .event(|event| matches!(event, Event::HostChanged { nick, username, host, .. } if nick == "bob" && username == "bob" && host == "users.example"))
        .server(":ergo.test 730 papo :ALICE!~alice@cloak.example")
        .no_event()
        .server(":ergo.test 731 papo :alice")
        .event(|event| matches!(event, Event::MonitorOffline { nick, .. } if nick == "alice" && nick.casemapping() == Casemapping::Ascii))
        .server(":ergo.test 731 papo :alice")
        .no_event()
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .server(":ergo.test 734 papo 100 carol :Monitor list is full")
        .event(|event| matches!(event, Event::MonitorListFull { limit: 100, nicks, .. } if nicks == &["carol"] && nicks[0].casemapping() == Casemapping::Ascii))
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
        .inspect(|session| {
            assert_eq!(
                session
                    .state()
                    .monitor_online()
                    .map(|user| user.nick().as_str())
                    .collect::<Vec<_>>(),
                ["bob"]
            );
        })
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .no_event()
        .server(":ergo.test 730 papo :bob!bob@users.example")
        .no_event();
}

#[test]
fn list_full_untracks_only_rejected_targets_from_a_batched_add() {
    registered()
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .client("MONITOR + alice,bob,carol")
        .server(":solanum.example 731 papo :alice")
        .event(|event| matches!(event, Event::MonitorOffline { nick, .. } if nick == "alice"))
        .server(":solanum.example 734 papo 1 bob,carol :Monitor list is full")
        .event(|event| matches!(event, Event::MonitorListFull { limit: 1, nicks, .. } if nicks == &["bob", "carol"]))
        .inspect(|session| {
            assert_eq!(
                session
                    .state()
                    .monitored()
                    .map(Nick::as_str)
                    .collect::<Vec<_>>(),
                ["alice"]
            );
        })
        .update(|session| session.monitor_add(["dave"]))
        .event(|event| matches!(event, Event::MonitorListFull { limit: 1, nicks, .. } if nicks == &["dave"]))
        .no_client();
}

#[test]
fn unsolicited_list_full_does_not_change_other_monitor_entries() {
    registered()
        .update(|session| session.monitor_add(["alice", "bob"]))
        .client("MONITOR + alice,bob")
        .server(":insp.example 731 papo :alice,bob")
        .event(|event| matches!(event, Event::MonitorOffline { nick, .. } if nick == "alice"))
        .event(|event| matches!(event, Event::MonitorOffline { nick, .. } if nick == "bob"))
        .server(":insp.example 734 papo 1 mallory :Monitor list is full")
        .event(
            |event| matches!(event, Event::MonitorListFull { nicks, .. } if nicks == &["mallory"]),
        )
        .inspect(|session| {
            assert_eq!(
                session
                    .state()
                    .monitored()
                    .map(Nick::as_str)
                    .collect::<Vec<_>>(),
                ["alice", "bob"]
            );
        })
        .update(|session| session.monitor_add(["carol"]))
        .client("MONITOR + carol");
}

#[test]
fn rejected_direct_watch_releases_its_monitor_slot() {
    let alice = RefCell::new(None);
    let bob = RefCell::new(None);
    registered()
        .update(|session| {
            *alice.borrow_mut() = Some(session.__direct_acquire(
                "alice",
                None,
                None,
                Account::Unknown,
            ));
        })
        .client("MONITOR + alice")
        .server(":solanum.example 734 papo 1 alice :Monitor list is full")
        .event(|event| matches!(event, Event::MonitorListFull { limit: 1, nicks, .. } if nicks == &["alice"]))
        .inspect(|session| {
            assert_eq!(session.state().monitored().count(), 0);
            assert!(session.state().user("alice").is_none());
        })
        .update(|session| {
            *bob.borrow_mut() = Some(session.__direct_acquire(
                "bob",
                None,
                None,
                Account::Unknown,
            ));
        })
        .client("MONITOR + bob")
        .server(":solanum.example 731 papo :bob")
        .event(|event| matches!(event, Event::MonitorOffline { nick, .. } if nick == "bob"))
        .update(|session| {
            session.__direct_release(alice.borrow().as_ref().unwrap());
            session.__direct_release(bob.borrow().as_ref().unwrap());
        })
        .client("MONITOR - bob")
        .no_client();
}

#[test]
fn slot_status_distinguishes_pending_confirmed_refused_and_disconnected() {
    Transcript::new(Config::default())
        .inspect(|session| assert_eq!(session.__monitor_limit(), None))
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome")
        .server(":ergo.test 005 papo MONITOR=2 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .update(|session| session.monitor_add(["alice", "bob"]))
        .client("MONITOR + alice,bob")
        .inspect(|session| {
            assert!(session.__monitor_reserves("alice"));
            assert!(!session.__monitor_confirms("alice"));
            assert_eq!(session.__monitor_reserved_len(), 2);
            assert_eq!(session.__monitor_limit(), Some(2));
        })
        .server(":ergo.test 731 papo :alice")
        .event(|event| matches!(event, Event::MonitorOffline { nick, .. } if nick == "alice"))
        .inspect(|session| {
            assert!(session.__monitor_confirms("alice"));
            assert!(!session.__monitor_confirms("bob"));
        })
        .server(":solanum.example 734 papo 1 bob :Monitor list is full")
        .event(|event| matches!(event, Event::MonitorListFull { nicks, .. } if nicks == &["bob"]))
        .inspect(|session| {
            assert!(!session.__monitor_reserves("bob"));
            assert_eq!(session.__monitor_reserved_len(), 1);
            assert_eq!(session.__monitor_limit(), Some(1));
        })
        .disconnect()
        .inspect(|session| {
            assert!(!session.__monitor_reserves("alice"));
            assert_eq!(session.__monitor_reserved_len(), 0);
            assert_eq!(session.__monitor_limit(), Some(2));
        });
}

#[test]
fn direct_acquisition_uses_only_an_available_confirmed_slot() {
    registered()
        .server(":ergo.test 005 papo MONITOR=2 :are supported")
        .update(|session| session.monitor_add(["alice"]))
        .client("MONITOR + alice")
        .update(|session| {
            session.__direct_acquire("alice", None, None, Account::Unknown);
        })
        .no_client();

    registered()
        .server(":ergo.test 005 papo MONITOR=2 :are supported")
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .client("MONITOR + alice,bob")
        .event(|event| matches!(event, Event::MonitorListFull { nicks, .. } if nicks == &["carol"]))
        .update(|session| {
            session.__direct_acquire("carol", None, None, Account::Unknown);
        })
        .no_client();

    registered()
        .server(":ergo.test 005 papo MONITOR=2 :are supported")
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .client("MONITOR + alice,bob")
        .event(|event| matches!(event, Event::MonitorListFull { nicks, .. } if nicks == &["carol"]))
        .server(":solanum.example 734 papo 2 alice :Monitor list is full")
        .event(|event| matches!(event, Event::MonitorListFull { nicks, .. } if nicks == &["alice"]))
        .update(|session| {
            session.__direct_acquire("carol", None, None, Account::Unknown);
        })
        .client("MONITOR + carol");
}

#[test]
fn direct_acquisition_and_removal_wait_for_registration() {
    let before_registration = || {
        Transcript::new(Config::default())
            .connect_default()
            .server(":ergo.test 005 * MONITOR=2 :are supported")
    };
    before_registration()
        .update(|session| session.monitor_add(["alice"]))
        .update(|session| {
            session.__direct_acquire("alice", None, None, Account::Unknown);
        })
        .no_client();
    before_registration()
        .update(|session| session.monitor_add(["alice", "bob"]))
        .update(|session| session.monitor_remove(["alice"]))
        .no_client();
}

#[test]
fn malformed_monitor_replies_remain_messages() {
    registered()
        .server(":ergo.test 730 papo :!alice@cloak.example")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().raw_command() == "730"))
        .server(":ergo.test 730 papo :alice!@cloak.example")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().raw_command() == "730"))
        .server(":ergo.test 730 papo :alice!~alice@")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().raw_command() == "730"))
        .server(":ergo.test 731 papo :")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().raw_command() == "731"))
        .server(":ergo.test 734 papo many alice :Monitor list is full")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().raw_command() == "734"))
        .server(":ergo.test 734 papo 100 :")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().raw_command() == "734"))
        .no_event();
}

#[test]
fn restores_the_desired_list_and_resets_presence_after_reconnect() {
    registered()
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .client("MONITOR + alice,bob,carol")
        .server(":ergo.test 731 papo :alice")
        .event(|event| matches!(event, Event::MonitorOffline { nick, .. } if nick == "alice"))
        .disconnect()
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome back to Ergo")
        .server(":ergo.test 005 papo MONITOR=100 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .client("MONITOR + alice,bob,carol")
        .server(":ergo.test 731 papo :alice")
        .event(|event| matches!(event, Event::MonitorOffline { nick, .. } if nick == "alice"))
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
        .welcome(":ergo.test 001 papo :Welcome back")
        .server(":ergo.test 005 papo MONITOR=100 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
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
            .welcome(":ergo.test 001 papo :Welcome")
            .server(":ergo.test 005 papo MONITOR=2 :are supported")
            .server(":ergo.test 376 papo :End of MOTD")
            .event(|event| matches!(event, Event::Connected(_)))
            .update(|session| session.monitor_add(nicks))
    };

    register(vec!["alice", "bob"])
        .client("MONITOR + alice,bob")
        .disconnect()
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome back")
        .server(":ergo.test 005 papo MONITOR=2 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .client("MONITOR + alice,bob")
        .no_event();

    register(vec!["alice", "bob", "carol"])
        .client("MONITOR + alice,bob")
        .event(|event| matches!(event, Event::MonitorListFull { limit: 2, nicks, .. } if nicks == &["carol"]))
        .disconnect()
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome back")
        .server(":ergo.test 005 papo MONITOR=2 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .client("MONITOR + alice,bob")
        .event(|event| matches!(event, Event::MonitorListFull { limit: 2, nicks, .. } if nicks == &["carol"]))
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
        .event(|event| matches!(event, Event::AwayChanged { nick, away: Away::Away { message: Some(message), .. }, .. } if nick == "alice" && message == "Gone to lunch"))
        .server(":alice!~alice@cloak.example ACCOUNT alice")
        .event(|event| matches!(event, Event::AccountChanged { nick, account, .. } if nick == "alice" && account == &Account::LoggedIn(Casemapping::default().account("alice"))))
        .server(":alice!~alice@cloak.example CHGHOST alice account/alice")
        .event(|event| matches!(event, Event::HostChanged { nick, .. } if nick == "alice"))
        .server(":alice!alice@account/alice SETNAME :Alice Smith")
        .event(|event| matches!(event, Event::RealnameChanged { nick, .. } if nick == "alice"))
        .inspect(|session| {
            let alice = session.state().user("alice").unwrap();
            assert_eq!(alice.channels().count(), 0);
            assert_eq!(
                alice.away(),
                &Away::__away_value(Some("Gone to lunch".into()))
            );
            assert_eq!(
                alice.account(),
                &Account::LoggedIn(Casemapping::default().account("alice"))
            );
            assert_eq!(alice.host(), Some("account/alice"));
            assert_eq!(alice.realname(), Some("Alice Smith"));
        })
        .server(":ergo.test 731 papo :alice")
        .event(|event| matches!(event,
            Event::MonitorOffline { nick, token, facts: Some(user), .. }
                if nick == "alice" && token.__is_known()
                    && user.host() == Some("account/alice")
                    && user.realname() == Some("Alice Smith")
        ))
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
#[allow(clippy::too_many_lines)]
fn departure_facts_follow_remaining_user_visibility() {
    #[derive(Clone, Copy)]
    enum Kind {
        Part,
        Kick,
        Monitor,
    }

    #[derive(Clone, Copy)]
    enum Visibility {
        Own,
        Shared,
        Monitored,
        Last,
        MonitorShared,
        MonitorOwn,
        MonitorUnknown,
    }

    const OWN_JOIN: &str = ":papo!papo@127.0.0.1 JOIN #papo";
    const SHARED: &[&str] = &[
        OWN_JOIN,
        ":papo!papo@127.0.0.1 JOIN #rust",
        ":alice!~alice@staff.example JOIN #papo",
        ":alice!~alice@staff.example JOIN #rust",
    ];
    const LAST: &[&str] = &[OWN_JOIN, ":alice!~alice@staff.example JOIN #papo"];
    let cases = [
        ("own part", Kind::Part, Visibility::Own, false),
        ("own kick", Kind::Kick, Visibility::Own, false),
        ("shared part", Kind::Part, Visibility::Shared, false),
        ("shared kick", Kind::Kick, Visibility::Shared, false),
        ("monitored part", Kind::Part, Visibility::Monitored, false),
        ("monitored kick", Kind::Kick, Visibility::Monitored, false),
        ("last-channel part", Kind::Part, Visibility::Last, true),
        ("last-channel kick", Kind::Kick, Visibility::Last, true),
        (
            "shared offline",
            Kind::Monitor,
            Visibility::MonitorShared,
            false,
        ),
        ("own offline", Kind::Monitor, Visibility::MonitorOwn, false),
        (
            "unknown offline",
            Kind::Monitor,
            Visibility::MonitorUnknown,
            true,
        ),
    ];

    for (name, kind, visibility, carries_facts) in cases {
        let own = matches!(visibility, Visibility::Own | Visibility::MonitorOwn);
        let watched = matches!(
            visibility,
            Visibility::Monitored
                | Visibility::MonitorShared
                | Visibility::MonitorOwn
                | Visibility::MonitorUnknown
        );
        let setup = match visibility {
            Visibility::Own => &[OWN_JOIN][..],
            Visibility::Shared => SHARED,
            Visibility::Monitored | Visibility::Last | Visibility::MonitorShared => LAST,
            Visibility::MonitorOwn | Visibility::MonitorUnknown => &[],
        };
        let departure = match (kind, own) {
            (Kind::Part, true) => ":papo!papo@127.0.0.1 PART #papo :Deploying release",
            (Kind::Part, false) => ":alice!~alice@staff.example PART #papo :Deploying release",
            (Kind::Kick, true) => ":ChanServ!service@services.example KICK #papo papo :Requested",
            (Kind::Kick, false) => ":ChanServ!service@services.example KICK #papo alice :Requested",
            (Kind::Monitor, true) => ":ergo.test 731 papo :papo",
            (Kind::Monitor, false) => ":ergo.test 731 papo :alice",
        };
        let mut transcript = registered();
        if watched {
            let nick = if own { "papo" } else { "alice" };
            transcript = transcript
                .update(|session| session.monitor_add([nick]))
                .client(&format!("MONITOR + {nick}"))
                .server(&format!(
                    ":ergo.test 730 papo :{nick}!~{nick}@staff.example"
                ));
        }
        for line in setup {
            transcript = transcript.server(line);
        }
        transcript
            .server(departure)
            .event_until(|event| {
                let actual = match (kind, event) {
                    (Kind::Part, Event::Parted { nick, facts, .. }) if (nick == "papo") == own => {
                        facts.is_some()
                    }
                    (Kind::Kick, Event::Kicked { subject, facts, .. })
                        if (subject == "papo") == own =>
                    {
                        facts.is_some()
                    }
                    (Kind::Monitor, Event::MonitorOffline { nick, facts, .. })
                        if (nick == "papo") == own =>
                    {
                        facts.is_some()
                    }
                    _ => return false,
                };
                assert_eq!(actual, carries_facts, "{name}");
                true
            })
            .ignore_rest();
    }
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
        .welcome(":soju.example 001 papo :Welcome")
        .server(":soju.example 005 papo MONITOR=100 :are supported")
        .advance(std::time::Duration::from_secs(9))
        .no_client()
        .advance(std::time::Duration::from_secs(1))
        .event(|event| matches!(event, Event::Connected(_)))
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
        .welcome(":solanum.example 001 papo :Welcome")
        .server(":solanum.example 422 papo :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
        .update(papo_core::Session::monitor_clear)
        .no_client()
        .update(|session| session.monitor_add(["alice"]))
        .no_event()
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
        .event(|event| matches!(event, Event::MonitorOffline { nick, .. } if nick == "alice"))
        .inspect(|session| {
            assert_eq!(session.state().monitor_online().count(), 0);
            assert_eq!(session.state().__monitor_presence("alice"), Some(false));
            assert!(session.state().user("alice").is_none());
        })
        .no_event();
}

#[test]
fn monitor_offline_retains_a_shared_user() {
    registered()
        .update(|session| session.monitor_add(["alice"]))
        .client("MONITOR + alice")
        .server(":papo!papo@127.0.0.1 JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "papo"))
        .server(":alice!~alice@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "alice"))
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .server(":ergo.test 731 papo :alice")
        .event(|event| matches!(event, Event::MonitorOffline { nick, .. } if nick == "alice"))
        .inspect(|session| {
            let alice = session.state().user("alice").unwrap();
            assert_eq!(
                alice
                    .channels()
                    .map(papo_proto::ChannelName::as_str)
                    .collect::<Vec<_>>(),
                ["#papo"]
            );
            assert_eq!(session.state().monitor_online().count(), 0);
        })
        .no_event();
}

#[test]
fn monitored_quit_and_nickname_arrival_reconcile_identity_state() {
    registered()
        .update(|session| session.monitor_add(["alice", "carol"]))
        .client("MONITOR + alice,carol")
        .server(":papo!papo@localhost JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "papo"))
        .server(":bob!~bob@users/bob JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "bob"))
        .server(":mallory!~mallory@users/mallory JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "mallory"))
        .server(":carol!~carol@users/carol JOIN #papo")
        .event(|event| matches!(event, Event::Joined { nick, .. } if nick == "carol"))
        .server(":ergo.test 730 papo :alice!~alice@users/alice,carol!~carol@users/carol")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "carol"))
        .server("@+typing=active :alice!~alice@users/alice TAGMSG papo")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .server(":alice!~alice@users/alice QUIT :Client exited")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .event(|event| matches!(event, Event::Quit { nick, .. } if nick == "alice"))
        .server(":ergo.test 005 papo NETWORK=Ergo :are supported")
        .inspect(|session| {
            let state = session.state();
            let conversation = papo_core::Conversation::Private(state.casemapping().nick("alice"));
            assert_eq!(state.monitor_online().count(), 1);
            assert_eq!(state.typing(&conversation).count(), 0);
            assert!(state.user("bob").is_some());
            assert!(state.user("mallory").is_some());
            assert_eq!(state.user("carol").unwrap().channels().count(), 1);
        })
        .server("@+typing=active :alice!~alice@users/alice TAGMSG papo")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .server(":mallory!~mallory@users/mallory NICK :alice")
        .event(|event| matches!(event, Event::NickChanged { new, .. } if new == "alice"))
        .server(":ergo.test 005 papo NETWORK=Ergo :are supported")
        .inspect(|session| {
            let state = session.state();
            let conversation = papo_core::Conversation::Private(state.casemapping().nick("alice"));
            assert_eq!(state.typing(&conversation).count(), 0);
            let online = state.monitor_online().collect::<Vec<_>>();
            assert_eq!(online.len(), 2);
            assert!(
                online
                    .iter()
                    .any(|user| user.nick() == "alice" && user.host() == Some("users/mallory"))
            );
            assert!(online.iter().any(|user| user.nick() == "carol"));
            assert!(state.user("bob").is_some());
            assert_eq!(state.user("carol").unwrap().channels().count(), 1);
        })
        .no_event();
}

#[test]
fn monitored_nick_change_marks_the_old_nickname_offline() {
    registered()
        .update(|session| session.monitor_add(["alice"]))
        .client("MONITOR + alice")
        .server(":ergo.test 730 papo :alice!~alice@users/alice")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .server(":alice!~alice@users/alice NICK :alicia")
        .event(|event| matches!(event, Event::NickChanged { new, .. } if new == "alicia"))
        .server(":ergo.test 005 papo NETWORK=Ergo :are supported")
        .inspect(|session| assert_eq!(session.state().monitor_online().count(), 0))
        .no_event();
}

#[test]
fn reports_unsupported_monitor_only_once() {
    Transcript::new(Config::default())
        .update(|session| session.monitor_add(["alice"]))
        .connect_default()
        .welcome(":solanum.example 001 papo :Welcome")
        .server(":solanum.example 422 papo :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
        .event(|event| {
            matches!(event, Event::Unsupported(unsupported)
            if unsupported.feature == Feature::Monitor
                && unsupported.reason == UnsupportedReason::NotOffered)
        })
        .no_client()
        .disconnect()
        .connect_default()
        .welcome(":solanum.example 001 papo :Welcome back")
        .server(":solanum.example 422 papo :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
        .no_event()
        .no_client();
}

#[test]
fn warns_when_monitor_limit_becomes_zero_after_reconnect() {
    registered()
        .update(|session| session.monitor_add(["alice"]))
        .client("MONITOR + alice")
        .disconnect()
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome back to Ergo")
        .server(":ergo.test 005 papo MONITOR=0 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .event(|event| {
            matches!(event, Event::Unsupported(unsupported)
            if unsupported.feature == Feature::Monitor
                && unsupported.reason == UnsupportedReason::LimitZero)
        })
        .no_client();
}

#[test]
fn caps_the_desired_monitor_list_at_ten_thousand() {
    let nicks = (0..10_001).map(|index| format!("user{index:05}"));
    Transcript::new(Config::default())
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome")
        .server(":ergo.test 005 papo MONITOR :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .update(|session| session.monitor_add(nicks))
        .event(|event| matches!(event, Event::MonitorListFull { limit: 10_000, nicks, .. } if nicks == &["user10000"]))
        .ignore_rest();
}

#[test]
fn rekeys_monitor_presence_when_casemapping_changes() {
    registered()
        .update(|session| session.monitor_add(["[alice"]))
        .client("MONITOR + [alice")
        .server(":ergo.test 730 papo :[alice!user@users.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "[alice"))
        .server(":ergo.test 005 papo CASEMAPPING=ascii :are supported")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .server(":ergo.test 730 papo :[alice!user@users.example")
        .no_event();
}

#[test]
fn presence_at_the_desired_bound_ignores_the_next_nick() {
    registered()
        .update(|session| session.monitor_add((0..10_000).map(|index| format!("user{index:05}"))))
        .update(|session| {
            for index in [0, 9_999, 10_000] {
                session.handle_line(
                    Instant::now(),
                    format!(":ergo.test 730 papo :user{index:05}!user@users.example").as_bytes(),
                );
                let _ = session.poll_event();
            }
            assert_eq!(session.state().monitor_online().count(), 2);
        })
        .ignore_rest();
}

#[test]
fn user_requested_monitor_list_replies_remain_messages() {
    registered()
        .send(Line::new("MONITOR").with_params(["L"]))
        .client("MONITOR L")
        .server(":ergo.test 732 papo :alice,bob")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().raw_command() == "732"))
        .server(":ergo.test 733 papo :End of MONITOR list")
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().raw_command() == "733"))
        .no_event();
}
