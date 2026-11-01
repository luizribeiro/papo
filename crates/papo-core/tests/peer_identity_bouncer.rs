//! Peer identity invalidation across a bouncer upstream session.

use std::time::Instant;

use papo_core::{Account, Config, RecipientGuard, RecipientStatus, Session};

fn replay(session: &mut Session, trace: &str) {
    let now = Instant::now();
    let mut original = None;
    let mut guard = None;
    let mut dropped = false;
    for raw in trace.lines() {
        if raw == "# upstream disabled" {
            let token = session.state().__peer_token_for("alice");
            assert!(token.__is_known());
            original = Some(token);
            guard = Some(RecipientGuard::__new(
                session.state(),
                session.state().casemapping().nick("alice"),
                Account::Unknown,
            ));
            continue;
        }
        let Some(line) = raw.strip_prefix("S: ") else {
            continue;
        };
        session.handle_line(now, line.as_bytes());
        if line.contains("disconnected from ergo") || line == "BOUNCER NETWORK 1 state=disconnected"
        {
            dropped = true;
            assert!(session.state().user("alice").is_none());
            assert!(!session.state().__peer_token_for("alice").__is_known());
            assert_eq!(
                session.state().__resolve_recipient(guard.as_ref().unwrap()),
                RecipientStatus::Gone
            );
        }
    }
    assert!(dropped);
    let replacement = session.state().__peer_token_for("alice");
    assert!(replacement.__is_known());
    assert_ne!(replacement, original.unwrap());
    assert_eq!(
        session.state().__resolve_recipient(guard.as_ref().unwrap()),
        RecipientStatus::Gone
    );
}

#[test]
fn stable_soju_notice_ends_peer_identity() {
    let mut session = Session::new(Config::default().nick("papo-bouncer"));
    replay(
        &mut session,
        include_str!("data/soju-upstream-drop-stable.irc"),
    );
}

#[cfg(feature = "unstable")]
#[test]
fn selected_bouncer_network_drop_ends_peer_identity() {
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
    replay(
        &mut session,
        include_str!("data/soju-upstream-drop-unstable.irc"),
    );
}
