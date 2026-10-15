//! Transcript tests for `draft/persistence`.

mod support;

use papo_core::{Config, Event, PersistenceSetting, PersistenceStatus};
use support::Transcript;

#[test]
fn stores_a_persistence_push_before_motd() {
    Transcript::new(Config::default().wanted_caps(["draft/persistence"]))
        .connect_default()
        .server("CAP * LS :draft/persistence")
        .client("CAP REQ :draft/persistence")
        .server("CAP * ACK :draft/persistence")
        .client("CAP END")
        .server(":ergo.test 001 alice :Welcome to Ergo")
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
