//! Transcript tests for `draft/metadata-2` and `draft/metadata-3`.

mod support;

use papo_core::{
    Config, Event, MetadataCommand, MetadataError, MetadataLimits, RequestError, StateLimit,
};
use std::time::Duration;
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
    .inspect(|session| assert!(session.state().metadata("alice").is_none()))
    .update(|session| {
        session.metadata(std::time::Instant::now(), MetadataCommand::Clear {
            target: "alice".into(),
        }).unwrap();
    })
    .client("@label=2 METADATA alice CLEAR")
    .server("@label=2 :ergo.test FAIL METADATA VALUE_INVALID :Metadata value is invalid")
    .event(|event| matches!(event, Event::Response(response)
        if response.metadata_error() == Some(MetadataError::ValueInvalid)));
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

#[test]
fn applies_a_join_metadata_batch_only_when_it_closes() {
    registered("max-subs=100,max-keys=100")
        .server(":ergo.test BATCH +m metadata #papo")
        .server("@batch=m :ergo.test 761 alice #papo avatar * :https://example.test/papo.png")
        .server("@batch=m :ergo.test 761 alice bob pronouns * :he/him")
        .no_event()
        .inspect(|session| assert!(session.state().metadata("#papo").is_none()))
        .server(":ergo.test BATCH -m")
        .event(|event| {
            matches!(event, Event::MetadataChanged { target, .. }
            if target == "#papo")
        })
        .event(|event| {
            matches!(event, Event::MetadataChanged { target, .. }
            if target == "bob")
        })
        .inspect(|session| {
            assert_eq!(session.state().metadata("#PAPO").unwrap().len(), 1);
            assert_eq!(session.state().metadata("BOB").unwrap().len(), 1);
        });
}

#[test]
fn tracks_sub_subs_and_unsub_replies() {
    registered("max-subs=2,max-keys=100")
        .update(|session| {
            session
                .metadata(
                    std::time::Instant::now(),
                    MetadataCommand::Sub {
                        keys: vec!["avatar".into(), "pronouns".into()],
                    },
                )
                .unwrap();
        })
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .client("METADATA * SUB avatar pronouns")
        .server(":ergo.test 770 alice avatar pronouns")
        .inspect(|session| {
            assert_eq!(
                session.state().metadata_subscriptions(),
                ["avatar", "pronouns"]
            );
        })
        .update(|session| {
            assert!(matches!(
                session.metadata(
                    std::time::Instant::now(),
                    MetadataCommand::Sub {
                        keys: vec!["status".into()],
                    },
                ),
                Err(RequestError::TooManyMetadataSubscriptions { actual: 3, .. })
            ));
        })
        .no_client()
        .update(|session| {
            session
                .metadata(std::time::Instant::now(), MetadataCommand::Subs)
                .unwrap();
        })
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .client("METADATA * SUBS")
        .server(":ergo.test BATCH +s metadata-subs")
        .server("@batch=s :ergo.test 772 alice avatar status")
        .server(":ergo.test BATCH -s")
        .inspect(|session| {
            assert_eq!(
                session.state().metadata_subscriptions(),
                ["avatar", "status"]
            );
        })
        .update(|session| {
            session
                .metadata(
                    std::time::Instant::now(),
                    MetadataCommand::Unsub {
                        keys: vec!["avatar".into(), "status".into(), "pronouns".into()],
                    },
                )
                .unwrap();
        })
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .client("METADATA * UNSUB avatar status pronouns")
        .server(":ergo.test 771 alice avatar status")
        .inspect(|session| assert!(session.state().metadata_subscriptions().is_empty()));
}

#[test]
fn types_contextual_metadata_failures() {
    for (line, expected) in [
        (
            ":ergo.test FAIL METADATA RATE_LIMITED alice avatar 45 :Metadata update rate limit exceeded",
            MetadataError::RateLimited {
                target: "alice".into(),
                key: "avatar".into(),
                retry_after: Some(Duration::from_secs(45)),
            },
        ),
        (
            ":ergo.test FAIL METADATA RATE_LIMITED alice pronouns * :Metadata update rate limit exceeded",
            MetadataError::RateLimited {
                target: "alice".into(),
                key: "pronouns".into(),
                retry_after: None,
            },
        ),
        (
            ":ergo.test FAIL METADATA TOO_MANY_SUBS status :Too many metadata subscriptions",
            MetadataError::TooManySubscriptions {
                first_unprocessed_key: "status".into(),
            },
        ),
    ] {
        registered("max-subs=100,max-keys=100")
            .server(line)
            .event(|event| {
                matches!(event, Event::StandardReply(reply)
                if reply.metadata_error() == Some(expected))
            });
    }
}

#[test]
fn types_each_simple_metadata_failure_only_for_metadata() {
    use MetadataError as E;
    for (code, expected) in [
        ("INVALID_TARGET", E::InvalidTarget),
        ("KEY_INVALID", E::KeyInvalid),
        ("KEY_NO_PERMISSION", E::KeyNoPermission),
        ("KEY_NOT_SET", E::KeyNotSet),
        ("LIMIT_REACHED", E::LimitReached),
        ("SUBCOMMAND_INVALID", E::SubcommandInvalid),
        ("VALUE_INVALID", E::ValueInvalid),
    ] {
        registered("max-subs=100,max-keys=100")
            .server(&format!(
                ":ergo.test FAIL METADATA {code} :Metadata request rejected"
            ))
            .event(|event| {
                matches!(event, Event::StandardReply(reply)
                if reply.metadata_error() == Some(expected))
            });
    }
    registered("max-subs=100,max-keys=100")
        .server(":ergo.test FAIL WHOIS KEY_INVALID :WHOIS request rejected")
        .event(|event| {
            matches!(event, Event::StandardReply(reply)
            if reply.metadata_error().is_none())
        });
}
