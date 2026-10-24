//! Transcript tests for `draft/metadata-2` and `draft/metadata-3`.

mod support;

use papo_core::{
    Config, Event, MetadataCommand, MetadataLimits, Refusal, RequestError, StateLimit, Subject,
    Visibility,
};
use std::time::Duration;
use support::Transcript;

fn subject_name(subject: &Subject) -> &str {
    match subject {
        Subject::Channel(channel) => channel,
        Subject::User(nick) => nick,
        _ => panic!("unexpected subject"),
    }
}

fn registered(value: &str) -> Transcript {
    Transcript::new(Config::default().wanted_caps(["draft/metadata-2"]))
        .connect_default()
        .server(&format!("CAP * LS :draft/metadata-2={value}"))
        .client("CAP REQ :draft/metadata-2")
        .server("CAP * ACK :draft/metadata-2")
        .client("CAP END")
        .welcome(":ergo.test 001 alice :Welcome to Ergo")
        .server(":ergo.test 422 alice :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
}

fn assert_cached_visibility(session: &papo_core::Session) {
    let metadata = session.state().metadata("alice").unwrap();
    assert_eq!(metadata["avatar"].value, "https://cdn.example.net/a.png");
    assert_eq!(metadata["avatar"].visibility, Visibility::Public);
    assert_eq!(metadata["pronouns"].value, "they/them");
    assert_eq!(
        metadata["pronouns"].visibility,
        Visibility::Restricted("channel".to_owned())
    );
}

#[test]
fn user_metadata_follows_a_shared_nick_change() {
    registered("max-subs=100,max-keys=100")
        .server(":alice!~alice@users/alice JOIN #papo")
        .server(":bob!~bob@users/bob JOIN #papo")
        .server(":ergo.test 761 alice bob display-name * :Bob Admin")
        .server(":bob!~bob@users/bob NICK :bob-away")
        .inspect(|session| {
            assert!(session.state().metadata("bob").is_none());
            assert_eq!(
                session.state().metadata("bob-away").unwrap()["display-name"].value,
                "Bob Admin"
            );
        })
        .ignore_rest();
}

#[test]
fn quit_metadata_is_not_inherited_by_a_new_nick_holder() {
    registered("max-subs=100,max-keys=100")
        .server(":alice!~alice@users/alice JOIN #papo")
        .server(":bob!~bob@users/bob JOIN #papo")
        .server(":mallory!~mallory@users/mallory JOIN #papo")
        .server(":ergo.test 761 alice bob avatar * :https://cdn.example.net/bob.png")
        .server(":bob!~bob@users/bob QUIT :Client exited")
        .server(":mallory!~mallory@users/mallory NICK :bob")
        .inspect(|session| assert!(session.state().metadata("bob").is_none()))
        .ignore_rest();
}

#[test]
fn metadata_lifetime_tracks_channels_monitor_and_snapshot_identity() {
    registered("max-subs=100,max-keys=100")
        .server(":alice!~alice@users/alice JOIN #papo")
        .server(":alice!~alice@users/alice JOIN #rust")
        .server(":bob!~bob@users/bob JOIN #papo")
        .server(":bob!~bob@users/bob JOIN #rust")
        .server(":ergo.test 761 alice #papo topic-label * :Release planning")
        .server(":ergo.test 761 alice bob display-name * :Bob Builder")
        .server(":alice!~alice@users/alice PART #papo :Leaving")
        .inspect(|session| {
            assert!(session.state().metadata("#papo").is_none());
            assert!(session.state().metadata("bob").is_some());
        })
        .server(":bob!~bob@users/bob PART #rust :Leaving")
        .inspect(|session| assert!(session.state().metadata("bob").is_none()))
        .ignore_rest();

    registered("max-subs=100,max-keys=100")
        .server(":ergo.test 005 alice MONITOR=100 :are supported")
        .update(|session| session.monitor_add(["bob"]))
        .client("MONITOR + bob")
        .server(":alice!~alice@users/alice JOIN #papo")
        .server(":bob!~bob@users/bob JOIN #papo")
        .server(":ergo.test 761 alice bob avatar * :https://cdn.example.net/bob.png")
        .server(":bob!~bob@users/bob PART #papo :Leaving")
        .inspect(|session| assert!(session.state().metadata("bob").is_some()))
        .ignore_rest();

    registered("max-subs=100,max-keys=100")
        .server(":ergo.test 761 alice alice display-name * :Alice Admin")
        .server(":alice!~alice@users/alice NICK :alice-away")
        .inspect(|session| {
            assert!(session.state().metadata("alice").is_none());
            assert!(session.state().metadata("alice-away").is_some());
        })
        .ignore_rest();

    registered("max-subs=100,max-keys=100")
        .server(":ergo.test 761 alice bob display-name * :Stale Snapshot")
        .server(":bob!~bob@users/bob NICK :bob-away")
        .inspect(|session| {
            assert!(session.state().metadata("bob").is_none());
            assert!(session.state().metadata("bob-away").is_none());
        })
        .ignore_rest();
}

#[test]
fn requests_published_metadata_name_and_reads_known_limits() {
    Transcript::new(Config::default().wanted_caps([
        "draft/metadata-2",
        "metadata-notify",
        "labeled-response",
    ]))
    .connect_default()
    .server("CAP * LS :metadata-notify draft/metadata-2=max-subs=9 draft/metadata-3=before-connect,max-subs=100,max-keys=100,max-value-bytes=4096,future=yes labeled-response")
    .client("CAP REQ :draft/metadata-2 labeled-response")
    .server("CAP * ACK :draft/metadata-2 labeled-response")
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
    .event(|event| matches!(event, Event::Connected(_)))
    .update(|session| {
        session
            .metadata(
                std::time::Instant::now(),
                MetadataCommand::__set("alice".into(), "avatar".into(), Some("https://example.com/alice.png".into())),
            )
            .unwrap();
    })
    .client("@label=0 METADATA alice SET avatar https://example.com/alice.png")
    .update(|session| {
        session
            .metadata(
                std::time::Instant::now(),
                MetadataCommand::__set("alice".into(), "pronouns".into(), Some("they/them".into())),
            )
            .unwrap();
    })
    .client("@label=1 METADATA alice SET pronouns they/them")
    .update(|session| {
        session
            .metadata(
                std::time::Instant::now(),
                MetadataCommand::__get("alice".into(), vec!["avatar".into(), "pronouns".into()]),
            )
            .unwrap();
    })
    .client("@label=2 METADATA alice GET avatar pronouns")
    .server("@label=0 :ergo.test 761 alice ALICE avatar * :https://cdn.example.net/a.png")
    .event(|event| matches!(event, Event::MetadataChanged { subject, key, value: Some(value), visibility, .. }
        if subject_name(subject) == "ALICE" && key == "avatar"
            && value == "https://cdn.example.net/a.png" && visibility == &Visibility::Public))
    .event(|event| matches!(event, Event::Response(_)))
    .server("@label=1 :ergo.test 761 alice ALICE pronouns channel :they/them")
    .event(|event| matches!(event, Event::MetadataChanged {
        key,
        visibility: Visibility::Restricted(policy),
        ..
    } if key == "pronouns" && policy == "channel"))
    .event(|event| matches!(event, Event::Response(_)))
    .server("@label=2 :ergo.test BATCH +reply labeled-response")
    .server("@batch=reply :ergo.test 761 alice ALICE avatar * :https://cdn.example.net/a.png")
    .server("@batch=reply :ergo.test 761 alice ALICE pronouns channel :they/them")
    .server(":ergo.test BATCH -reply")
    .event(|event| matches!(event, Event::MetadataChanged { key, .. } if key == "avatar"))
    .event(|event| matches!(event, Event::MetadataChanged { key, .. } if key == "pronouns"))
    .event(|event| matches!(event, Event::Response(_)))
    .inspect(assert_cached_visibility)
    .server(":ergo.test 766 alice ALICE avatar :key is not set")
    .event(|event| matches!(event, Event::MetadataChanged { key, value: None, .. }
        if key == "avatar"))
    .inspect(|session| assert!(!session.state().metadata("alice").unwrap().contains_key("avatar")))
    .update(|session| {
        session.metadata(std::time::Instant::now(), MetadataCommand::__clear("alice".into())).unwrap();
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
                    MetadataCommand::__set(
                        "alice".into(),
                        "a0_./-".into(),
                        Some("12345678".into()),
                    ),
                )
                .unwrap();
            session
                .metadata(
                    now,
                    MetadataCommand::__sub(vec!["avatar".into(), "pronouns".into()]),
                )
                .unwrap();
            assert_eq!(
                session.metadata(
                    now,
                    MetadataCommand::__set("alice".into(), "status".into(), Some(String::new())),
                ),
                Err(RequestError::EmptyText)
            );
            assert_eq!(
                session.metadata(
                    now,
                    MetadataCommand::__get("alice".into(), vec!["Avatar".into()]),
                ),
                Err(RequestError::__invalid_metadata_key(
                    "Avatar".into(),
                    papo_proto::InvalidIdentifier::InvalidCharacter('A'),
                ))
            );
            assert_eq!(
                session.metadata(
                    now,
                    MetadataCommand::__set(
                        "alice".into(),
                        "pronouns".into(),
                        Some("they/them!".into())
                    ),
                ),
                Err(RequestError::__metadata_value_too_long(8, 10))
            );
            assert_eq!(
                session.metadata(
                    now,
                    MetadataCommand::__sub(vec![
                        "avatar".into(),
                        "pronouns".into(),
                        "status".into()
                    ]),
                ),
                Err(RequestError::__too_many_metadata_subscriptions(2, 3))
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
            matches!(event, Event::MetadataChanged { subject, key, .. }
            if subject_name(subject) == "#Papo" && key == "soju.im/pinned")
        })
        .server(":soju.example METADATA #papo soju.im/muted * :1")
        .event(|event| {
            matches!(event, Event::MetadataChanged { subject, key, .. }
            if subject_name(subject) == "#papo" && key == "soju.im/muted")
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
                    limit: StateLimit::MetadataKeys,
                    ..
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
            matches!(event, Event::MetadataChanged { subject, .. }
            if subject_name(subject) == "#papo")
        })
        .event(|event| {
            matches!(event, Event::MetadataChanged { subject, .. }
            if subject_name(subject) == "bob")
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
                    MetadataCommand::__sub(vec!["avatar".into(), "pronouns".into()]),
                )
                .unwrap();
        })
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .client("METADATA * SUB avatar pronouns")
        .server(":ergo.test 770 alice avatar pronouns")
        .inspect(|session| {
            assert!(
                session
                    .state()
                    .metadata_subscriptions()
                    .map(AsRef::as_ref)
                    .eq(["avatar", "pronouns"])
            );
        })
        .update(|session| {
            assert!(matches!(
                session.metadata(
                    std::time::Instant::now(),
                    MetadataCommand::__sub(vec!["status".into()]),
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
            assert!(
                session
                    .state()
                    .metadata_subscriptions()
                    .map(AsRef::as_ref)
                    .eq(["avatar", "status"])
            );
        })
        .update(|session| {
            session
                .metadata(
                    std::time::Instant::now(),
                    MetadataCommand::__unsub(vec![
                        "avatar".into(),
                        "status".into(),
                        "pronouns".into(),
                    ]),
                )
                .unwrap();
        })
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .client("METADATA * UNSUB avatar status pronouns")
        .server(":ergo.test 771 alice avatar status")
        .inspect(|session| {
            assert!(session.state().metadata_subscriptions().next().is_none());
        });
}

#[test]
fn unsubscribe_reply_removes_only_the_acknowledged_key() {
    registered("max-subs=100,max-keys=100")
        .server(":ergo.test 770 alice avatar pronouns status")
        .server(":ergo.test 771 alice pronouns")
        .inspect(|session| {
            assert!(
                session
                    .state()
                    .metadata_subscriptions()
                    .map(AsRef::as_ref)
                    .eq(["avatar", "status"])
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
            matches!(event, Event::MetadataSyncLater { target, retry_after, .. }
            if target == "#papo" && *retry_after == Some(Duration::from_secs(90)))
        })
        .server(":ergo.test 774 alice bob")
        .event(|event| {
            matches!(event, Event::MetadataSyncLater { target, retry_after: None, .. }
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
        .event(|event| matches!(event, Event::Connected(_)))
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
fn subscribes_after_connect_without_before_connect() {
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
        .event(|event| matches!(event, Event::Connected(_)))
        .client("@label=0 METADATA * SUB avatar");
}

#[test]
fn encodes_list_remove_clear_and_sync() {
    registered("max-subs=100,max-keys=100")
        .update(|session| {
            let now = std::time::Instant::now();
            for command in [
                MetadataCommand::__list("alice".into()),
                MetadataCommand::__set("alice".into(), "status".into(), None),
                MetadataCommand::__clear("alice".into()),
                MetadataCommand::__sync("#papo".into()),
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
