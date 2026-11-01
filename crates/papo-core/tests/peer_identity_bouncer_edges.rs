//! Boundary coverage for bouncer upstream identity signals.

use std::time::Instant;

use papo_core::{Config, Session};

#[test]
fn duplicate_own_join_is_a_stable_bouncer_resync_signal() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo-bouncer"));
    let mut original = None;
    let mut upstream_disabled = false;
    for raw in include_str!("data/soju-upstream-drop-stable.irc").lines() {
        if raw == "# upstream disabled" {
            original = Some(session.state().__peer_token_for("alice"));
            upstream_disabled = true;
            continue;
        }
        if raw == "# upstream enabled" {
            upstream_disabled = false;
            continue;
        }
        if upstream_disabled {
            continue;
        }
        let Some(line) = raw.strip_prefix("S: ") else {
            continue;
        };
        session.handle_line(now, line.as_bytes());
        if original.is_some() && line.contains(" JOIN #review") {
            break;
        }
    }
    assert!(session.state().user("alice").is_none());
    assert_ne!(session.state().__peer_token_for("alice"), original.unwrap());
}

#[test]
fn nearby_notices_and_ordinary_duplicate_joins_preserve_identity() {
    for signal in [
        ":BouncerServ!service@BouncerServ NOTICE papo-bouncer :disconnected from ergo",
        ":BouncerServ!BouncerServ@BouncerServ NOTICE somebody-else :disconnected from ergo",
        ":BouncerServ!BouncerServ@BouncerServ NOTICE papo-bouncer :connected to ergo",
        ":papo-bouncer!~u@client.example JOIN #review",
    ] {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo-bouncer"));
        for line in [
            ":papo-bouncer!~u@client.example JOIN #review",
            ":alice!~alice@cloak.example JOIN #review",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        let original = session.state().__peer_token_for("alice");
        session.handle_line(now, signal.as_bytes());
        assert_eq!(session.state().__peer_token_for("alice"), original);
    }
}

#[test]
fn one_resync_with_multiple_channels_only_ends_identity_once() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo-bouncer"));
    for line in [
        ":bouncer.papo.test 005 * BOUNCER_NETID=1 CASEMAPPING=ascii :are supported",
        ":papo-bouncer!~u@old.example JOIN #review",
        ":papo-bouncer!~u@old.example JOIN #ops",
        ":alice!alice-old@cloak.example JOIN #review",
        ":BouncerServ!BouncerServ@BouncerServ NOTICE papo-bouncer :disconnected from ergo",
        ":papo-bouncer!~u@new.example JOIN #review",
        ":bouncer.papo.test 353 * = #review :@alice papo-bouncer",
        ":bouncer.papo.test 366 * #review :End of /NAMES list",
        ":papo-bouncer!~u@new.example JOIN #ops",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    assert!(session.state().user("alice").is_some());

    session.handle_line(now, b":bouncer.papo.test 324 papo-bouncer #review +Cnt");
    let current = session.state().__peer_token_for("alice");
    session.handle_line(now, b":papo-bouncer!~u@later.example JOIN #review");
    assert_ne!(session.state().__peer_token_for("alice"), current);
}

#[cfg(feature = "unstable")]
#[test]
fn only_selected_network_states_outside_connected_end_identity() {
    for (line, changes) in [
        ("BOUNCER NETWORK 2 state=disconnected", false),
        ("BOUNCER NETWORK 1 state=connected", false),
        ("BOUNCER NETWORK 1 state=connecting", true),
        ("BOUNCER NETWORK 1 state=disconnected", true),
        ("BOUNCER NETWORK 1 *", true),
    ] {
        let now = Instant::now();
        let mut session = Session::new(
            Config::default()
                .nick("papo-bouncer")
                .enable(papo_core::Feature::Bouncer)
                .wanted_caps(["soju.im/bouncer-networks"]),
        );
        session.handle_connected(now);
        session.handle_line(now, b"CAP * LS :soju.im/bouncer-networks");
        session.handle_line(now, b"CAP * ACK :soju.im/bouncer-networks");
        for setup in [
            ":bouncer.papo.test 005 * BOUNCER_NETID=1 CASEMAPPING=ascii :are supported",
            ":papo-bouncer!~u@client.example JOIN #review",
            ":alice!~alice@cloak.example JOIN #review",
        ] {
            session.handle_line(now, setup.as_bytes());
        }
        let original = session.state().__peer_token_for("alice");
        session.handle_line(now, line.as_bytes());
        assert_eq!(
            session.state().__peer_token_for("alice") != original,
            changes,
            "{line}"
        );
    }
}
