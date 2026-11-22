//! Peer identity invalidation across a bouncer upstream session.

mod support;

use std::time::{Duration, Instant};

use papo_core::{AccountStatus, Config, Event, RecipientStatus, Session};
use papo_proto::Casemapping;
use support::Transcript;

fn replay(session: &mut Session, trace: &str) {
    let now = Instant::now();
    let mut original = None;
    let mut guard = None;
    let mut dropped = false;
    for raw in trace.lines() {
        if raw == "# upstream disabled" {
            let token = session.state().__reply_token_for("alice");
            assert!(token.__is_known());
            original = Some(token);
            guard = session.state().__capture_recipient(
                session.state().casemapping().nick("alice"),
                AccountStatus::Unknown,
            );
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
            assert!(!session.state().__reply_token_for("alice").__is_known());
            let remaining = session
                .state()
                .channel("#review")
                .unwrap()
                .members()
                .map(|member| member.nick().as_str())
                .collect::<Vec<_>>();
            assert_eq!(remaining, ["papo-bouncer"]);
            assert_eq!(
                session.state().__resolve_recipient(guard.as_ref().unwrap()),
                RecipientStatus::Gone
            );
        }
    }
    assert!(dropped);
    let replacement = session.state().__reply_token_for("alice");
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

#[test]
fn selected_bouncer_network_drop_ends_peer_identity() {
    let mut session = Session::new(Config::default().nick("papo-bouncer"));
    replay(
        &mut session,
        include_str!("data/soju-upstream-drop-unstable.irc"),
    );
}

#[test]
fn late_soju_isupport_establishes_name_rules_after_welcome() {
    let upstream_isupport = include_str!("data/soju-upstream-drop-stable.irc")
        .lines()
        .filter_map(|line| line.strip_prefix("S: "))
        .find(|line| line.starts_with(":bouncer.papo.test 005 "))
        .unwrap();

    Transcript::new(Config::default().nick("papo-bouncer"))
        .connect()
        .client("CAP LS 302")
        .client("NICK papo-bouncer")
        .client("USER papo 0 * papo")
        .server(":bouncer.papo.test CAP * LS :")
        .client("CAP END")
        .welcome(":bouncer.papo.test 001 papo-bouncer :Welcome to soju, papo-bouncer")
        .server(
            ":bouncer.papo.test 005 papo-bouncer BOUNCER_NETID=1 CHATHISTORY=1000 :are supported",
        )
        .server(":bouncer.papo.test 422 papo-bouncer :Use /motd to read the message of the day")
        .event(|event| matches!(event, Event::Connected(_)))
        .inspect(|session| assert!(!session.__name_rules_known()))
        .no_event()
        .advance(Duration::from_secs(1))
        .server(upstream_isupport)
        .event(|event| event == &Event::__casemapping_changed(None, Casemapping::Ascii))
        .inspect(|session| {
            assert!(session.__name_rules_known());
            assert_eq!(session.state().casemapping(), Casemapping::Ascii);
        })
        .event(|event| matches!(event, Event::FeaturesChanged { .. }))
        .no_event()
        .no_client();
}
