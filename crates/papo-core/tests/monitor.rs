//! Transcript tests for `IRCv3` MONITOR state.

mod support;

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
}

#[test]
fn tracks_online_offline_and_list_full_replies() {
    registered()
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .client("MONITOR + alice,bob,carol")
        .server(":ergo.test 730 papo :alice!~alice@cloak.example,bob!bob@users.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, user, host } if nick == "alice" && user == "~alice" && host == "cloak.example"))
        .event(|event| matches!(event, Event::MonitorOnline { nick, user, host } if nick == "bob" && user == "bob" && host == "users.example"))
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
fn extended_monitor_updates_an_unshared_users_record() {
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
