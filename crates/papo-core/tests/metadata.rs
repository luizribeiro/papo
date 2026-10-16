//! Transcript tests for `draft/metadata-2` and `draft/metadata-3`.

mod support;

use papo_core::{Config, Event, MetadataCommand, MetadataLimits, RequestError, StateLimit};
use support::Transcript;

fn registered(value: &str) -> Transcript {
    Transcript::new(Config::default().wanted_caps(["draft/metadata-2"]))
        .connect_default()
        .server(&format!("CAP * LS :draft/metadata-2={value}"))
        .client("CAP REQ :draft/metadata-2")
        .server("CAP * ACK :draft/metadata-2")
        .client("CAP END")
        .server(":ergo.test 001 alice :Welcome to Ergo")
        .server(":ergo.test 422 alice :MOTD File is missing")
        .event(|event| matches!(event, Event::Registered(_)))
}

#[test]
fn prefers_metadata_3_and_reads_known_limits() {
    Transcript::new(Config::default().wanted_caps([
        "draft/metadata-2",
        "metadata-notify",
        "labeled-response",
    ]))
    .connect_default()
    .server("CAP * LS :metadata-notify draft/metadata-2=max-subs=9 draft/metadata-3=before-connect,max-subs=100,max-keys=100,max-value-bytes=4096,future=yes labeled-response")
    .client("CAP REQ :draft/metadata-3 labeled-response")
    .server("CAP * ACK :draft/metadata-3 labeled-response")
    .client("CAP END")
    .inspect(|session| {
        assert_eq!(
            session.metadata_limits(),
            Some(MetadataLimits {
                before_connect: true,
                max_subscriptions: Some(100),
                max_keys: Some(100),
                max_value_bytes: Some(4096),
            })
        );
    })
    .server(":ergo.test 001 alice :Welcome to Ergo")
    .server(":ergo.test 422 alice :MOTD File is missing")
    .event(|event| matches!(event, Event::Registered(_)))
    .update(|session| {
        session
            .metadata(
                std::time::Instant::now(),
                MetadataCommand::Set {
                    target: "alice".into(),
                    key: "avatar".into(),
                    value: Some("https://example.com/alice.png".into()),
                },
            )
            .unwrap();
    })
    .client("@label=0 METADATA alice SET avatar https://example.com/alice.png")
    .update(|session| {
        session
            .metadata(
                std::time::Instant::now(),
                MetadataCommand::Get {
                    target: "alice".into(),
                    keys: vec!["avatar".into(), "pronouns".into()],
                },
            )
            .unwrap();
    })
    .client("@label=1 METADATA alice GET avatar pronouns")
    .server("@label=0 :ergo.test 761 alice ALICE avatar * :https://cdn.example.net/a.png")
    .event(|event| matches!(event, Event::MetadataChanged { target, key, value: Some(value), visibility }
        if target == "ALICE" && key == "avatar"
            && value == "https://cdn.example.net/a.png" && visibility == "*"))
    .event(|event| matches!(event, Event::Response(_)))
    .inspect(|session| {
        let avatar = &session.state().metadata("alice").unwrap()["avatar"];
        assert_eq!(avatar.value, "https://cdn.example.net/a.png");
        assert_eq!(avatar.visibility, "*");
    })
    .server("@label=1 :ergo.test 766 alice ALICE avatar :key is not set")
    .event(|event| matches!(event, Event::MetadataChanged { key, value: None, .. }
        if key == "avatar"))
    .event(|event| matches!(event, Event::Response(_)))
    .inspect(|session| assert!(session.state().metadata("alice").is_none()));
}

#[test]
fn falls_back_to_metadata_2() {
    Transcript::new(Config::default().wanted_caps(["draft/metadata-3"]))
        .connect_default()
        .server("CAP * LS :draft/metadata-2=max-subs=100,max-keys=100")
        .client("CAP REQ :draft/metadata-2")
        .server("CAP * ACK :draft/metadata-2")
        .client("CAP END")
        .inspect(|session| {
            assert_eq!(session.metadata_limits().unwrap().max_keys, Some(100));
        });
}

#[test]
fn rejects_invalid_keys_values_and_subscription_counts_locally() {
    registered("max-subs=2,max-keys=100,max-value-bytes=8")
        .update(|session| {
            let now = std::time::Instant::now();
            session
                .metadata(
                    now,
                    MetadataCommand::Set {
                        target: "alice".into(),
                        key: "a0_./-".into(),
                        value: Some("12345678".into()),
                    },
                )
                .unwrap();
            session
                .metadata(
                    now,
                    MetadataCommand::Sub {
                        keys: vec!["avatar".into(), "pronouns".into()],
                    },
                )
                .unwrap();
            assert_eq!(
                session.metadata(
                    now,
                    MetadataCommand::Get {
                        target: "alice".into(),
                        keys: vec!["Avatar".into()],
                    },
                ),
                Err(RequestError::InvalidMetadataKey)
            );
            assert_eq!(
                session.metadata(
                    now,
                    MetadataCommand::Set {
                        target: "alice".into(),
                        key: "pronouns".into(),
                        value: Some("they/them!".into()),
                    },
                ),
                Err(RequestError::MetadataValueTooLong {
                    maximum: 8,
                    actual: 10,
                })
            );
            assert_eq!(
                session.metadata(
                    now,
                    MetadataCommand::Sub {
                        keys: vec!["avatar".into(), "pronouns".into(), "status".into()],
                    },
                ),
                Err(RequestError::TooManyMetadataSubscriptions {
                    maximum: 2,
                    actual: 3,
                })
            );
        })
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .client("METADATA alice SET a0_./- 12345678")
        .client("METADATA * SUB avatar pronouns")
        .no_client();
}

#[test]
fn caches_pushed_updates_up_to_the_advertised_key_limit() {
    registered("max-subs=100,max-keys=2")
        .server(":bob!b@example.test METADATA #Papo avatar * :https://example.test/bob.png")
        .event(|event| {
            matches!(event, Event::MetadataChanged { target, key, .. }
            if target == "#Papo" && key == "avatar")
        })
        .server(":bob!b@example.test METADATA #papo pronouns members :they/them")
        .event(|event| {
            matches!(event, Event::MetadataChanged { visibility, .. }
            if visibility == "members")
        })
        .server(":bob!b@example.test METADATA #papo status * :working")
        .event(|event| {
            matches!(
                event,
                Event::StateLimitReached {
                    limit: StateLimit::MetadataKeys,
                }
            )
        })
        .inspect(|session| {
            let metadata = session.state().metadata("#PAPO").unwrap();
            assert_eq!(metadata.len(), 2);
            assert!(!metadata.contains_key("status"));
        });
}
