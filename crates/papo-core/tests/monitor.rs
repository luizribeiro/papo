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
