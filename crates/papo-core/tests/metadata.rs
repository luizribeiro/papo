//! Transcript tests for `draft/metadata-2` and `draft/metadata-3`.

mod support;

use papo_core::{
    Config, Event, MetadataCommand, MetadataLimits, Refusal, RequestError, StateLimit,
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
        .welcome(":ergo.test 001 alice :Welcome to Ergo")
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
    .welcome(":ergo.test 001 alice :Welcome to Ergo")
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
                MetadataCommand::Set {
                    target: "alice".into(),
                    key: "pronouns".into(),
                    value: Some("they/them".into()),
                },
            )
            .unwrap();
    })
    .client("@label=1 METADATA alice SET pronouns they/them")
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
    .client("@label=2 METADATA alice GET avatar pronouns")
    .server("@label=0 :ergo.test 761 alice ALICE avatar * :https://cdn.example.net/a.png")
    .event(|event| matches!(event, Event::MetadataChanged { target, key, value: Some(value), visibility }
        if target == "ALICE" && key == "avatar"
            && value == "https://cdn.example.net/a.png" && visibility == "*"))
    .event(|event| matches!(event, Event::Response(_)))
    .server("@label=1 :ergo.test 761 alice ALICE pronouns * :they/them")
    .event(|event| matches!(event, Event::MetadataChanged { key, .. } if key == "pronouns"))
    .event(|event| matches!(event, Event::Response(_)))
    .server("@label=2 :ergo.test BATCH +reply labeled-response")
    .server("@batch=reply :ergo.test 761 alice ALICE avatar * :https://cdn.example.net/a.png")
    .server("@batch=reply :ergo.test 761 alice ALICE pronouns * :they/them")
    .server(":ergo.test BATCH -reply")
    .event(|event| matches!(event, Event::MetadataChanged { key, .. } if key == "avatar"))
    .event(|event| matches!(event, Event::MetadataChanged { key, .. } if key == "pronouns"))
    .event(|event| matches!(event, Event::Response(_)))
    .inspect(|session| {
        let metadata = session.state().metadata("alice").unwrap();
        assert_eq!(metadata["avatar"].value, "https://cdn.example.net/a.png");
        assert_eq!(metadata["avatar"].visibility, "*");
        assert_eq!(metadata["pronouns"].value, "they/them");
    })
    .server(":ergo.test 766 alice ALICE avatar :key is not set")
    .event(|event| matches!(event, Event::MetadataChanged { key, value: None, .. }
        if key == "avatar"))
    .inspect(|session| assert!(!session.state().metadata("alice").unwrap().contains_key("avatar")))
    .update(|session| {
        session.metadata(std::time::Instant::now(), MetadataCommand::Clear {
            target: "alice".into(),
        }).unwrap();
    })
    .client("@label=3 METADATA alice CLEAR")
    .server("@label=3 :ergo.test FAIL METADATA VALUE_INVALID :Metadata value is invalid")
    .event(|event| matches!(event, Event::Response(response)
        if response.refusal() == Some(Refusal::InvalidMetadataValue)));
}

#[test]
fn keeps_legacy_metadata_when_modern_metadata_is_not_both_wanted_and_available() {
    Transcript::new(Config::default().wanted_caps(["metadata-notify"]))
        .connect_default()
        .server("CAP * LS :metadata-notify draft/metadata-3")
        .client("CAP REQ :metadata-notify");

    Transcript::new(Config::default().wanted_caps(["metadata-notify", "draft/metadata-3"]))
        .connect_default()
        .server("CAP * LS :metadata-notify")
        .client("CAP REQ :metadata-notify");
}

#[test]
fn cap_new_keeps_legacy_metadata_without_a_modern_preference() {
    registered("max-subs=100,max-keys=100")
        .server("CAP alice NEW :metadata-notify")
        .no_client();

    Transcript::new(Config::default().wanted_caps(["metadata-notify"]))
        .connect_default()
        .server("CAP * LS :draft/metadata-3")
        .client("CAP END")
        .server("CAP papo NEW :metadata-notify")
        .client("CAP REQ :metadata-notify");

    Transcript::new(Config::default().wanted_caps(["metadata-notify", "draft/metadata-3"]))
        .connect_default()
        .server("CAP * LS :")
        .client("CAP END")
        .server("CAP papo NEW :metadata-notify")
        .client("CAP REQ :metadata-notify");
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
                Err(RequestError::InvalidMetadataKey {
                    key: "Avatar".into(),
                    reason: papo_proto::InvalidIdentifier::InvalidCharacter('A'),
                })
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
fn advertised_max_keys_does_not_limit_received_metadata() {
    registered("before-connect,max-subs=100,max-keys=0,max-value-bytes=1")
        .server(":soju.example METADATA #Papo soju.im/pinned * :1")
        .event(|event| {
            matches!(event, Event::MetadataChanged { target, key, .. }
            if target == "#Papo" && key == "soju.im/pinned")
        })
        .server(":soju.example METADATA #papo soju.im/muted * :1")
        .event(|event| {
            matches!(event, Event::MetadataChanged { target, key, .. }
            if target == "#papo" && key == "soju.im/muted")
        })
        .inspect(|session| {
            let metadata = session.state().metadata("#PAPO").unwrap();
            assert_eq!(metadata.len(), 2);
            assert_eq!(metadata["soju.im/pinned"].value, "1");
            assert_eq!(metadata["soju.im/muted"].value, "1");
        });
}

#[test]
fn metadata_target_spelling_survives_casemapping_changes() {
    registered("max-subs=100,max-keys=100")
        .server(":soju.example METADATA #Papo[dev] soju.im/pinned * :1")
        .event(|event| matches!(event, Event::MetadataChanged { .. }))
        .server(":ergo.test 005 alice CASEMAPPING=ascii :are supported")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .inspect(|session| {
            assert_eq!(
                session.state().metadata("#Papo[dev]").unwrap()["soju.im/pinned"].value,
                "1"
            );
            assert!(session.state().metadata("#papo{dev}").is_none());
        });
}

#[test]
fn bounds_received_metadata_with_its_own_storage_limit() {
    registered("max-subs=100,max-keys=20000").update(|session| {
        for index in 0..10_001 {
            session.handle_line(
                std::time::Instant::now(),
                format!(":soju.example METADATA #papo vendor/key{index} * :enabled").as_bytes(),
            );
        }

        assert_eq!(session.state().metadata("#papo").unwrap().len(), 10_000);
        assert!(
            std::iter::from_fn(|| session.poll_event()).any(|event| matches!(
                event,
                Event::StateLimitReached {
                    limit: StateLimit::MetadataKeys
                }
            ))
        );
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
        .server("@batch=s :ergo.test 772 alice avatar")
        .server("@batch=s :ergo.test 772 alice status")
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
fn unsubscribe_reply_removes_only_the_acknowledged_key() {
    registered("max-subs=100,max-keys=100")
        .server(":ergo.test 770 alice avatar pronouns status")
        .server(":ergo.test 771 alice pronouns")
        .inspect(|session| {
            assert_eq!(
                session.state().metadata_subscriptions(),
                ["avatar", "status"]
            );
        });
}

#[test]
fn types_contextual_metadata_failures() {
    for (line, expected, context) in [
        (
            ":ergo.test FAIL METADATA RATE_LIMITED alice avatar 45 :Metadata update rate limit exceeded",
            Refusal::RateLimited,
            &["alice", "avatar", "45"][..],
        ),
        (
            ":ergo.test FAIL METADATA RATE_LIMITED alice pronouns * :Metadata update rate limit exceeded",
            Refusal::RateLimited,
            &["alice", "pronouns", "*"][..],
        ),
        (
            ":ergo.test FAIL METADATA TOO_MANY_SUBS status :Too many metadata subscriptions",
            Refusal::TooManyMetadataSubscriptions,
            &["status"][..],
        ),
    ] {
        registered("max-subs=100,max-keys=100")
            .server(line)
            .event(|event| {
                matches!(event, Event::StandardReply(reply)
                if reply.refusal() == Some(expected) && reply.context == context)
            });
    }
}

#[test]
fn types_each_simple_metadata_failure_only_for_metadata() {
    for (code, expected) in [
        ("INVALID_TARGET", Refusal::InvalidTarget),
        ("KEY_INVALID", Refusal::InvalidMetadataKey),
        ("KEY_NO_PERMISSION", Refusal::MetadataPermissionDenied),
        ("KEY_NOT_SET", Refusal::MetadataKeyNotSet),
        ("LIMIT_REACHED", Refusal::MetadataLimitReached),
        ("SUBCOMMAND_INVALID", Refusal::InvalidSubcommand),
        ("VALUE_INVALID", Refusal::InvalidMetadataValue),
    ] {
        registered("max-subs=100,max-keys=100")
            .server(&format!(
                ":ergo.test FAIL METADATA {code} :Metadata request rejected"
            ))
            .event(|event| {
                matches!(event, Event::StandardReply(reply)
                if reply.refusal() == Some(expected))
            });
    }
    registered("max-subs=100,max-keys=100")
        .server(":ergo.test FAIL WHOIS KEY_INVALID :WHOIS request rejected")
        .event(|event| {
            matches!(event, Event::StandardReply(reply)
            if reply.refusal() == Some(Refusal::Other))
        });
}

#[test]
fn reports_metadata_sync_later_with_and_without_a_retry_delay() {
    registered("max-subs=100,max-keys=100")
        .server(":ergo.test 774 alice #papo 90")
        .event(|event| {
            matches!(event, Event::MetadataSyncLater { target, retry_after }
            if target == "#papo" && *retry_after == Some(Duration::from_secs(90)))
        })
        .server(":ergo.test 774 alice bob")
        .event(|event| {
            matches!(event, Event::MetadataSyncLater { target, retry_after: None }
            if target == "bob")
        });
}

#[test]
fn subscribes_before_cap_end_and_resubscribes_with_an_empty_cache() {
    let config = Config::default()
        .wanted_caps(["draft/metadata-2", "labeled-response"])
        .metadata_subscriptions(["avatar", "pronouns"]);
    Transcript::new(config)
        .connect_default()
        .server(
            "CAP * LS :draft/metadata-2=before-connect,max-subs=100,max-keys=100 labeled-response",
        )
        .client("CAP REQ :draft/metadata-2 labeled-response")
        .server("CAP * ACK :draft/metadata-2 labeled-response")
        .client("@label=0 METADATA * SUB avatar pronouns")
        .client("CAP END")
        .welcome(":ergo.test 001 alice :Welcome to Ergo")
        .server(":ergo.test 422 alice :MOTD File is missing")
        .event(|event| matches!(event, Event::Registered(_)))
        .server("@label=0 :ergo.test 770 alice avatar pronouns")
        .event(|event| matches!(event, Event::Response(_)))
        .server(":bob!b@example.test METADATA alice avatar * :https://example.test/a.png")
        .event(|event| matches!(event, Event::MetadataChanged { .. }))
        .inspect(|session| assert!(session.state().metadata("alice").is_some()))
        .disconnect()
        .inspect(|session| assert!(session.state().metadata("alice").is_none()))
        .connect_default()
        .server(
            "CAP * LS :draft/metadata-2=before-connect,max-subs=100,max-keys=100 labeled-response",
        )
        .client("CAP REQ :draft/metadata-2 labeled-response")
        .server("CAP * ACK :draft/metadata-2 labeled-response")
        .client("@label=1 METADATA * SUB avatar pronouns")
        .client("CAP END");
}

#[test]
fn subscribes_after_registration_without_before_connect() {
    let config = Config::default()
        .wanted_caps(["draft/metadata-2", "labeled-response"])
        .metadata_subscriptions(["avatar"]);
    Transcript::new(config)
        .connect_default()
        .server("CAP * LS :draft/metadata-2=max-subs=100,max-keys=100 labeled-response")
        .client("CAP REQ :draft/metadata-2 labeled-response")
        .server("CAP * ACK :draft/metadata-2 labeled-response")
        .client("CAP END")
        .welcome(":ergo.test 001 alice :Welcome to Ergo")
        .no_client()
        .server(":ergo.test 422 alice :MOTD File is missing")
        .event(|event| matches!(event, Event::Registered(_)))
        .client("@label=0 METADATA * SUB avatar");
}

#[test]
fn encodes_list_remove_clear_and_sync() {
    registered("max-subs=100,max-keys=100")
        .update(|session| {
            let now = std::time::Instant::now();
            for command in [
                MetadataCommand::List {
                    target: "alice".into(),
                },
                MetadataCommand::Set {
                    target: "alice".into(),
                    key: "status".into(),
                    value: None,
                },
                MetadataCommand::Clear {
                    target: "alice".into(),
                },
                MetadataCommand::Sync {
                    target: "#papo".into(),
                },
            ] {
                session.metadata(now, command).unwrap();
            }
        })
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .client("METADATA alice LIST")
        .client("METADATA alice SET status")
        .client("METADATA alice CLEAR")
        .client("METADATA #papo SYNC");
}
