//! Transcript tests for `IRCv3` MONITOR state.

mod support;

use papo_core::Config;
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
fn honors_the_advertised_monitor_limit_before_chunking() {
    let nicks: Vec<_> = (0..150).map(|index| format!("user{index:03}")).collect();
    let lines = [0..62, 62..100].map(|range| format!("MONITOR + {}", nicks[range].join(",")));
    registered()
        .update(|session| session.monitor_add(nicks))
        .client(&lines[0])
        .client(&lines[1])
        .no_client();
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
