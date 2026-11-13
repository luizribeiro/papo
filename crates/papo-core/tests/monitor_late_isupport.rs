//! Transcript tests for MONITOR support that changes after the connection finished.

mod support;

use std::time::Duration;

use papo_core::{Config, Event, Feature, UnsupportedReason};
use support::Transcript;

fn connected_without_monitor() -> Transcript {
    Transcript::new(Config::default())
        .update(|session| session.__reconcile_watches(vec!["alice".into(), "bob".into()]))
        .connect_default()
        .welcome(":soju.example 001 papo :Welcome to soju, papo")
        .server(":soju.example 005 papo CHANTYPES=# NETWORK=soju :are supported")
        .server(":soju.example 422 papo :Use /motd to read the message of the day")
        .event(|event| matches!(event, Event::Connected(_)))
        .no_client()
}

fn monitor_feature(added: bool) -> impl Fn(&Event) -> bool {
    move |event| {
        matches!(event, Event::FeaturesChanged { added: on, removed: off, .. }
            if (if added { on } else { off }) == &[Feature::Monitor])
    }
}

fn unsupported(reason: UnsupportedReason) -> impl Fn(&Event) -> bool {
    move |event| {
        matches!(event, Event::Unsupported(unsupported)
            if unsupported.feature == Feature::Monitor && unsupported.reason == reason)
    }
}

#[test]
fn late_monitor_advertisement_sends_the_watch_list_once() {
    connected_without_monitor()
        .server(":soju.example 005 papo MONITOR=100 WHOX :are supported")
        .client("MONITOR + alice,bob")
        .no_client()
        .server(":soju.example 005 papo MONITOR=100 WHOX :are supported")
        .no_client()
        .server(":soju.example 730 papo :alice!~alice@cloak.example")
        .event_until(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .server(":soju.example 731 papo :bob")
        .event(|event| matches!(event, Event::MonitorOffline { nick, .. } if nick == "bob"))
        .inspect(|session| {
            assert_eq!(session.state().__monitor_presence("alice"), Some(true));
            assert_eq!(session.state().__monitor_presence("bob"), Some(false));
        });
}

#[test]
fn late_monitor_advertisement_after_the_welcome_timeout_sends_the_list() {
    Transcript::new(Config::default())
        .update(|session| session.monitor_add(["alice", "bob"]))
        .connect_default()
        .welcome(":soju.example 001 papo :Welcome to soju, papo")
        .advance(Duration::from_secs(10))
        .event(|event| matches!(event, Event::Connected(_)))
        .event(unsupported(UnsupportedReason::NotOffered))
        .no_client()
        .server(":soju.example 005 papo MONITOR=100 :are supported")
        .client("MONITOR + alice,bob")
        .event_until(monitor_feature(true))
        .no_event()
        .no_client();
}

#[test]
fn late_monitor_limit_of_one_reports_the_rest_as_full() {
    connected_without_monitor()
        .server(":soju.example 005 papo MONITOR=1 :are supported")
        .client("MONITOR + alice")
        .event_until(|event| {
            matches!(event, Event::MonitorListFull { limit: 1, nicks, .. } if nicks == &["bob"])
        })
        .event(monitor_feature(true))
        .no_event()
        .no_client();
}

#[test]
fn monitor_withdrawn_after_connecting_warns_without_sending() {
    for withdrawal in [
        (
            ":ergo.test 005 papo -MONITOR :are supported",
            UnsupportedReason::NotOffered,
        ),
        (
            ":ergo.test 005 papo MONITOR=0 :are supported",
            UnsupportedReason::LimitZero,
        ),
    ] {
        Transcript::new(Config::default())
            .update(|session| session.monitor_add(["alice"]))
            .connect_default()
            .welcome(":ergo.test 001 papo :Welcome to Ergo")
            .server(":ergo.test 005 papo MONITOR=100 :are supported")
            .server(":ergo.test 376 papo :End of MOTD")
            .event(|event| matches!(event, Event::Connected(_)))
            .client("MONITOR + alice")
            .server(":ergo.test 730 papo :alice!~alice@cloak.example")
            .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
            .server(withdrawal.0)
            .event_until(unsupported(withdrawal.1))
            .event(monitor_feature(false))
            .no_event()
            .no_client()
            .inspect(|session| assert_eq!(session.state().__monitor_presence("alice"), None));
    }
}

#[test]
fn reconnecting_with_monitor_waits_for_the_connection_to_finish() {
    connected_without_monitor()
        .disconnect()
        .connect_default()
        .welcome(":soju.example 001 papo :Welcome to soju, papo")
        .server(":soju.example 005 papo MONITOR=100 :are supported")
        .no_client()
        .server(":soju.example 422 papo :Use /motd to read the message of the day")
        .event_until(|event| matches!(event, Event::Connected(_)))
        .client("MONITOR + alice,bob")
        .no_client()
        .ignore_rest();
}

#[test]
fn a_changed_monitor_limit_rebalances_the_list_and_keeps_presence() {
    Transcript::new(Config::default())
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 005 papo MONITOR=2 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .client("MONITOR + alice,bob")
        .event(|event| {
            matches!(event, Event::MonitorListFull { limit: 2, nicks, .. } if nicks == &["carol"])
        })
        .server(":ergo.test 730 papo :alice!~alice@cloak.example")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "alice"))
        .server(":ergo.test 005 papo MONITOR=3 :are supported")
        .client("MONITOR + carol")
        .no_client()
        .no_event()
        .server(":ergo.test 005 papo MONITOR=1 :are supported")
        .client("MONITOR - bob,carol")
        .event(|event| {
            matches!(event, Event::MonitorListFull { limit: 1, nicks, .. }
                if nicks == &["bob", "carol"])
        })
        .no_client()
        .no_event()
        .inspect(|session| {
            assert_eq!(session.state().__monitor_presence("alice"), Some(true));
            assert_eq!(session.state().__monitor_presence("bob"), None);
        });
}

#[test]
fn a_raised_monitor_limit_supersedes_a_list_full_reply() {
    Transcript::new(Config::default())
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 005 papo MONITOR=3 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .update(|session| session.monitor_add(["alice", "bob", "carol"]))
        .client("MONITOR + alice,bob,carol")
        .server(":ergo.test 734 papo 2 carol :Monitor list is full.")
        .event(|event| {
            matches!(event, Event::MonitorListFull { limit: 2, nicks, .. } if nicks == &["carol"])
        })
        .server(":ergo.test 005 papo MONITOR=5 :are supported")
        .no_client()
        .update(|session| session.monitor_add(["dave"]))
        .client("MONITOR + dave")
        .no_client()
        .no_event();
}
