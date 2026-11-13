//! Transcript tests for a server that advertises `MONITOR=0`, a list that accepts no targets.

mod support;

use papo_core::{Config, Event, Feature, Session, UnsupportedReason};
use support::Transcript;

type Operation = fn(&mut Session);

fn connected(token: &str, preset: bool) -> Transcript {
    Transcript::new(Config::default())
        .update(move |session| {
            if preset {
                session.monitor_add(["alice"]);
            }
        })
        .connect_default()
        .welcome(":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo")
        .server(&format!(
            ":ergo.test 005 papo {token} :are supported by this server"
        ))
        .server(":ergo.test 376 papo :End of MOTD command")
        .event_until(|event| matches!(event, Event::Connected(_)))
}

#[test]
fn monitor_operations_send_nothing_when_the_limit_is_zero() {
    let cases: [(bool, Operation, &str); 4] = [
        (
            false,
            |session| session.monitor_add(["alice"]),
            "MONITOR + alice",
        ),
        (
            false,
            |session| session.__reconcile_watches(vec!["alice".into()]),
            "MONITOR + alice",
        ),
        (
            true,
            |session| session.monitor_remove(["alice"]),
            "MONITOR - alice",
        ),
        (true, Session::monitor_clear, "MONITOR C"),
    ];
    for (preset, operation, line) in cases {
        for token in ["MONITOR", "MONITOR=100"] {
            let ready = connected(token, preset);
            let ready = if preset {
                ready.client("MONITOR + alice")
            } else {
                ready
            };
            ready
                .update(operation)
                .client(line)
                .no_client()
                .ignore_rest();
        }
        connected("MONITOR=0", preset)
            .no_client()
            .update(operation)
            .no_client()
            .ignore_rest();
    }
}

#[test]
fn connecting_with_a_zero_limit_warns_once_and_sends_nothing() {
    connected("MONITOR=0", true)
        .event(|event| {
            matches!(event, Event::Unsupported(unsupported)
                if unsupported.feature == Feature::Monitor
                    && unsupported.reason == UnsupportedReason::LimitZero)
        })
        .no_client()
        .update(|session| session.monitor_add(["bob"]))
        .no_client()
        .no_event();
}
