//! Transcript tests for `draft/persistence`.
#![cfg(feature = "unstable")]

mod support;

use papo_core::{Config, Event, PersistenceSetting, PersistenceStatus};
use support::Transcript;

fn registered(caps: &[&str]) -> Transcript {
    Transcript::new(Config::default().wanted_caps(caps.iter().copied()))
        .connect_default()
        .server(&format!("CAP * LS :{}", caps.join(" ")))
        .client(&format!("CAP REQ :{}", caps.join(" ")))
        .server(&format!("CAP * ACK :{}", caps.join(" ")))
        .client("CAP END")
        .welcome(":ergo.test 001 alice :Welcome to Ergo")
        .server(":ergo.test 422 alice :MOTD File is missing")
        .event(|event| matches!(event, Event::Registered(_)))
}

#[test]
fn stores_a_persistence_push_before_motd() {
    Transcript::new(Config::default().wanted_caps(["draft/persistence"]))
        .connect_default()
        .server("CAP * LS :draft/persistence")
        .client("CAP REQ :draft/persistence")
        .server("CAP * ACK :draft/persistence")
        .client("CAP END")
        .welcome(":ergo.test 001 alice :Welcome to Ergo")
        .server(":ergo.test PERSISTENCE STATUS DEFAULT ON")
        .event(|event| {
            matches!(
                event,
                Event::PersistenceStatus {
                    setting: PersistenceSetting::Default,
                    effective: true,
                }
            )
        })
        .inspect(|session| {
            assert_eq!(
                session.state().persistence(),
                Some(PersistenceStatus {
                    setting: PersistenceSetting::Default,
                    effective: true,
                })
            );
            assert!(session.server_rejoins_channels());
        })
        .server(":ergo.test 376 alice :End of MOTD")
        .event(|event| matches!(event, Event::Registered(_)));
}

#[test]
fn gets_persistence_status_in_a_labelled_response() {
    registered(&["draft/persistence", "labeled-response"])
        .update(|session| session.persistence_get(std::time::Instant::now()).unwrap())
        .client("@label=0 PERSISTENCE GET")
        .server("@label=0 :ergo.test PERSISTENCE STATUS OFF OFF")
        .event(|event| {
            matches!(
                event,
                Event::PersistenceStatus {
                    setting: PersistenceSetting::Off,
                    effective: false,
                }
            )
        })
        .event(|event| matches!(event, Event::Response(_)))
        .inspect(|session| assert!(!session.server_rejoins_channels()));
}

#[test]
fn sets_each_persistence_preference() {
    for (setting, token, effective) in [
        (PersistenceSetting::On, "ON", true),
        (PersistenceSetting::Off, "OFF", false),
        (PersistenceSetting::Default, "DEFAULT", true),
    ] {
        registered(&["draft/persistence", "labeled-response"])
            .update(|session| {
                session
                    .persistence_set(std::time::Instant::now(), setting)
                    .unwrap();
            })
            .client(&format!("@label=0 PERSISTENCE SET {token}"))
            .server(&format!(
                "@label=0 :ergo.test PERSISTENCE STATUS {token} {}",
                if effective { "ON" } else { "OFF" }
            ))
            .event(|event| matches!(event, Event::PersistenceStatus { setting: actual, effective: actual_effective }
                if *actual == setting && *actual_effective == effective))
            .event(|event| matches!(event, Event::Response(_)));
    }
}

#[test]
fn reports_persistence_account_required() {
    registered(&["draft/persistence"])
        .update(|session| session.persistence_get(std::time::Instant::now()).unwrap())
        .client("PERSISTENCE GET")
        .server(":ergo.test FAIL PERSISTENCE ACCOUNT_REQUIRED :An account is required")
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .event(|event| {
            matches!(event, Event::StandardReply(reply)
            if reply.command == "PERSISTENCE" && reply.code == "ACCOUNT_REQUIRED")
        });
}

#[test]
fn ignores_status_with_the_wrong_command_or_subcommand() {
    for line in [
        ":ergo.test NOTICE STATUS DEFAULT ON",
        ":ergo.test PERSISTENCE GET DEFAULT ON",
    ] {
        registered(&["draft/persistence"])
            .server(line)
            .event(|event| matches!(event, Event::Message(_)))
            .inspect(|session| assert_eq!(session.state().persistence(), None));
    }
}
