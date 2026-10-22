//! Transcript tests for optional feature predicates.
#![cfg(feature = "unstable")]

mod support;

use papo_core::{
    Config, Event, Feature, HistoryQuery, MetadataCommand, PersistenceSetting, ResponseMatcher,
    Session, Unsupported, UnsupportedReason,
};
use papo_proto::Line;
use support::{Transcript, unsupported_because};

const CAPS: &str = "message-tags draft/message-redaction draft/multiline draft/chathistory draft/read-marker draft/metadata-2 setname draft/account-registration soju.im/bouncer-networks draft/persistence draft/webpush draft/channel-rename draft/relaymsg labeled-response away-notify account-notify account-tag chghost invite-notify";

fn enable_extensions(config: Config) -> Config {
    config
        .enable(Feature::Bouncer)
        .enable(Feature::Persistence)
        .enable(Feature::WebPush)
        .enable(Feature::Relay)
}

fn session(caps: &str) -> Session {
    session_with(
        enable_extensions(Config::default()).wanted_caps(caps.split_whitespace()),
        caps,
        "",
    )
}

fn session_with(config: Config, caps: &str, isupport: &str) -> Session {
    let mut session = Session::new(config);
    let now = std::time::Instant::now();
    session.handle_connected(now);
    session.handle_line(now, format!(":ergo.test CAP papo LS :{caps}").as_bytes());
    session.handle_line(now, format!(":ergo.test CAP papo ACK :{caps}").as_bytes());
    if !isupport.is_empty() {
        session.handle_line(now, isupport.as_bytes());
    }
    session
}

#[test]
fn recognizes_every_capability_backed_feature() {
    let session = session(CAPS);
    for feature in [
        Feature::Replies,
        Feature::Reactions,
        Feature::Typing,
        Feature::Redaction,
        Feature::Multiline,
        Feature::History,
        Feature::ReadMarkers,
        Feature::Metadata,
        Feature::SetName,
        Feature::AccountCreation,
        Feature::Bouncer,
        Feature::Persistence,
        Feature::WebPush,
        Feature::ChannelRename,
        Feature::Relay,
        Feature::LabeledResponse,
        Feature::AwayTracking,
        Feature::AccountTracking,
        Feature::HostTracking,
        Feature::InviteTracking,
    ] {
        assert_eq!(session.supports(feature), Ok(()), "{feature:?}");
    }
}

#[test]
fn unstable_capabilities_require_explicit_feature_selection() {
    let offered = "soju.im/bouncer-networks soju.im/bouncer-networks-notify draft/persistence draft/relaymsg draft/webpush";
    Transcript::new(Config::default())
        .connect_default()
        .server(&format!("CAP * LS :{offered}"))
        .client("CAP END")
        .inspect(|session| {
            assert_eq!(
                session.supports(Feature::Bouncer),
                Err(UnsupportedReason::DisabledByClient)
            );
        })
        .server("CAP papo NEW :draft/webpush")
        .no_client();

    Transcript::new(
        Config::default()
            .enable(Feature::Bouncer)
            .enable(Feature::Persistence)
            .enable(Feature::Relay)
            .enable(Feature::WebPush),
    )
    .connect_default()
    .server(&format!("CAP * LS :{offered}"))
    .client(&format!("CAP REQ :{offered}"))
    .server(&format!("CAP * ACK :{offered}"))
    .client("CAP END");
}

#[test]
fn combines_isupport_tokens_with_negotiated_caps() {
    let session = session_with(
        Config::default().wanted_caps(["draft/chathistory"]),
        "draft/chathistory",
        ":irc.unreal.example 005 papo MONITOR=100 BOT=B CHATHISTORY=100 :are supported by this server",
    );
    for feature in [Feature::History, Feature::Monitor, Feature::BotMode] {
        assert_eq!(session.supports(feature), Ok(()), "{feature:?}");
    }
}

#[test]
fn recognizes_moderation_features_from_server_advertisements() {
    let cases = [
        (
            ":ergo.test 005 papo CHANMODES=Ibe,k,fl,CEMRUimnstu EXCEPTS EXTBAN=,m INVEX PREFIX=(qaohv)~&@%+ :are supported",
            [true, true, true, false, false],
        ),
        (
            ":solanum.example 005 papo ACCOUNTEXTBAN=a CHANMODES=eIbq,k,flj,CFLPQcgimnprstz EXCEPTS EXTBAN=$,a INVEX PREFIX=(ov)@+ :are supported",
            [false, true, true, true, true],
        ),
        (
            ":irc.unreal.example 005 papo ACCOUNTEXTBAN=account,a CHANMODES=beI,fkL,lFH,cdimnprstzCDGKMNOPQRSTVZ EXCEPTS EXTBAN=~,acfijmnpqrtACFGOST INVEX PREFIX=(qaohv)~&@%+ :are supported",
            [true, true, true, false, true],
        ),
        (
            ":irc.inspircd.example 005 papo ACCOUNTEXTBAN=account,R CHANMODES=b,k,l,MRimnprst EXTBAN=,RU PREFIX=(ov)@+ :are supported",
            [false, false, false, false, true],
        ),
    ];
    let features = [
        Feature::Halfop,
        Feature::Excepts,
        Feature::Invex,
        Feature::Quiet,
        Feature::AccountExtban,
    ];

    for (line, expected) in cases {
        let session = session_with(Config::default(), "", line);
        for (feature, supported) in features.into_iter().zip(expected) {
            assert_eq!(session.supports(feature).is_ok(), supported, "{feature:?}");
        }
    }
}

#[test]
fn account_tracking_requires_both_ratified_capabilities() {
    let only_notify = session_with(
        Config::default().wanted_caps(["account-notify"]),
        "account-notify",
        "",
    );
    assert_eq!(
        only_notify.supports(Feature::AccountTracking),
        Err(UnsupportedReason::NotOffered)
    );

    let both = session_with(
        Config::default().wanted_caps(["account-notify", "account-tag"]),
        "account-notify account-tag",
        "",
    );
    assert_eq!(both.supports(Feature::AccountTracking), Ok(()));
}

#[test]
fn rejects_literal_zero_feature_limits() {
    let zero = session_with(
        Config::default().wanted_caps(["draft/chathistory"]),
        "draft/chathistory",
        ":irc.unreal.example 005 papo MONITOR=0 CHATHISTORY=0 :are supported by this server",
    );
    assert_eq!(
        zero.supports(Feature::Monitor),
        Err(papo_core::UnsupportedReason::LimitZero)
    );
    assert_eq!(zero.supports(Feature::History), Ok(()));
}

#[test]
fn distinguishes_client_disabled_and_server_absent_features() {
    let features = [
        Feature::Replies,
        Feature::Reactions,
        Feature::Typing,
        Feature::Redaction,
        Feature::Multiline,
        Feature::History,
        Feature::ReadMarkers,
        Feature::Metadata,
        Feature::SetName,
        Feature::AccountCreation,
        Feature::Bouncer,
        Feature::Persistence,
        Feature::WebPush,
        Feature::ChannelRename,
        Feature::Relay,
        Feature::LabeledResponse,
        Feature::AwayTracking,
        Feature::AccountTracking,
        Feature::HostTracking,
        Feature::InviteTracking,
    ];
    let disabled = session_with(
        enable_extensions(Config::default())
            .wanted_caps(CAPS.split_whitespace())
            .without_caps(CAPS.split_whitespace()),
        CAPS,
        "",
    );
    let absent = session_with(
        enable_extensions(Config::default()).wanted_caps(CAPS.split_whitespace()),
        "",
        "",
    );
    for feature in features {
        assert_eq!(
            disabled.supports(feature),
            Err(papo_core::UnsupportedReason::DisabledByClient),
            "{feature:?}"
        );
        assert_eq!(
            absent.supports(feature),
            Err(papo_core::UnsupportedReason::NotOffered),
            "{feature:?}"
        );
    }
    for feature in [Feature::Monitor, Feature::BotMode] {
        assert_eq!(
            absent.supports(feature),
            Err(papo_core::UnsupportedReason::NotOffered)
        );
    }
}

#[test]
fn feature_names_support_display_and_hashing() {
    let mut features = std::collections::HashSet::new();

    assert!(features.insert(Feature::InviteTracking));
    assert!(!features.insert(Feature::InviteTracking));
    assert_eq!(Feature::InviteTracking.to_string(), "invite tracking");
    assert_eq!(
        Feature::InviteTracking.to_string(),
        Feature::InviteTracking.name()
    );
}

#[test]
fn recognizes_one_disabled_capability_when_it_is_offered() {
    let session = session_with(
        Config::default()
            .wanted_caps(["draft/metadata-3"])
            .without_caps(["draft/metadata-3"]),
        "draft/metadata-3",
        "",
    );
    assert_eq!(
        session.supports(Feature::Metadata),
        Err(UnsupportedReason::DisabledByClient)
    );
}

#[test]
fn applies_client_tag_denials_per_feature() {
    let denied = session_with(
        Config::default().wanted_caps(["message-tags"]),
        "message-tags",
        ":irc.solanum.example 005 papo CLIENTTAGDENY=reply,draft/reply,draft/react,typing :are supported by this server",
    );
    for feature in [Feature::Replies, Feature::Reactions, Feature::Typing] {
        assert_eq!(
            denied.supports(feature),
            Err(papo_core::UnsupportedReason::TagDenied),
            "{feature:?}"
        );
    }

    let draft_reply = session_with(
        Config::default().wanted_caps(["message-tags"]),
        "message-tags",
        ":irc.solanum.example 005 papo CLIENTTAGDENY=reply :are supported by this server",
    );
    assert_eq!(draft_reply.supports(Feature::Replies), Ok(()));
    assert_eq!(draft_reply.supports(Feature::Reactions), Ok(()));

    let no_reply_for_reactions = session_with(
        Config::default().wanted_caps(["message-tags"]),
        "message-tags",
        ":irc.solanum.example 005 papo CLIENTTAGDENY=reply,draft/reply :are supported by this server",
    );
    assert_eq!(
        no_reply_for_reactions.supports(Feature::Reactions),
        Err(UnsupportedReason::TagDenied)
    );
}

#[test]
fn unsupported_messages_name_the_feature_and_reason() {
    for (reason, expected) in [
        (UnsupportedReason::NotOffered, "the server did not offer it"),
        (
            UnsupportedReason::DisabledByClient,
            "it was disabled by the client",
        ),
        (
            UnsupportedReason::TagDenied,
            "the server denies its client-only tag",
        ),
        (
            UnsupportedReason::LimitZero,
            "the server advertised a zero limit",
        ),
    ] {
        assert_eq!(
            Unsupported::new(Feature::Reactions, reason).to_string(),
            format!("reactions unavailable: {expected}")
        );
    }
}

#[test]
fn reports_capability_and_isupport_feature_changes_after_connect() {
    Transcript::new(Config::default().wanted_caps(["message-tags"]))
        .connect_default()
        .server("CAP * LS :message-tags")
        .client("CAP REQ :message-tags")
        .server("CAP * ACK :message-tags")
        .client("CAP END")
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 005 papo MONITOR=100 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .server("CAP papo DEL :message-tags")
        .event(|event| matches!(event, Event::CapsChanged { .. }))
        .event(|event| {
            matches!(event, Event::FeaturesChanged { added, removed, .. }
            if added.is_empty()
                && removed == &[
                    Feature::MessageTags,
                    Feature::Replies,
                    Feature::Reactions,
                    Feature::Typing,
                ])
        })
        .server("CAP papo NEW :message-tags")
        .client("CAP REQ :message-tags")
        .server("CAP papo ACK :message-tags")
        .event(|event| matches!(event, Event::CapsChanged { .. }))
        .event(|event| {
            matches!(event, Event::FeaturesChanged { added, removed, .. }
                if added == &[
                    Feature::MessageTags,
                    Feature::Replies,
                    Feature::Reactions,
                    Feature::Typing,
                ]
                    && removed.is_empty())
        })
        .server(":irc.solanum.example 005 papo CLIENTTAGDENY=draft/react :are supported")
        .event(|event| {
            matches!(event, Event::FeaturesChanged { added, removed, .. }
                if added.is_empty() && removed == &[Feature::Reactions])
        })
        .server(":irc.solanum.example 005 papo -CLIENTTAGDENY :are supported")
        .event(|event| {
            matches!(event, Event::FeaturesChanged { added, removed, .. }
                if added == &[Feature::Reactions] && removed.is_empty())
        })
        .server(":irc.unreal.example 005 papo MONITOR=0 :are supported")
        .event(|event| {
            matches!(event, Event::FeaturesChanged { added, removed, .. }
                if added.is_empty() && removed == &[Feature::Monitor])
        })
        .server(":irc.unreal.example 005 papo MONITOR=100 :are supported")
        .event(|event| {
            matches!(event, Event::FeaturesChanged { added, removed, .. }
                if added == &[Feature::Monitor] && removed.is_empty())
        })
        .no_event();
}

#[test]
fn fails_a_pending_request_when_its_feature_disappears() {
    Transcript::new(Config::default().wanted_caps(["draft/chathistory", "labeled-response"]))
        .connect_default()
        .server("CAP * LS :draft/chathistory labeled-response")
        .client("CAP REQ :draft/chathistory labeled-response")
        .server("CAP * ACK :draft/chathistory labeled-response")
        .client("CAP END")
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 005 papo CHATHISTORY=100 :are supported")
        .server(":ergo.test 376 papo :End of MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .update(|session| {
            session
                .chathistory(
                    std::time::Instant::now(),
                    HistoryQuery::__latest("#papo".into(), None, 50),
                )
                .unwrap();
        })
        .client("@label=0 CHATHISTORY LATEST #papo * 50")
        .server("CAP papo DEL :draft/chathistory")
        .event(|event| matches!(event, Event::CapsChanged { .. }))
        .event(|event| {
            matches!(event, Event::FeaturesChanged { added, removed, .. }
                if added.is_empty() && removed == &[Feature::History])
        })
        .event(|event| {
            matches!(event, Event::RequestUnsupported { unsupported, .. }
                if unsupported.feature == Feature::History
                    && unsupported.reason == UnsupportedReason::NotOffered)
        })
        .no_event();
}

#[test]
fn failing_a_feature_request_keeps_other_join_tracking() {
    Transcript::new(
        Config::default()
            .nick("papo")
            .wanted_caps(["draft/chathistory", "labeled-response"]),
    )
    .connect_default()
    .server("CAP * LS :draft/chathistory labeled-response")
    .client("CAP REQ :draft/chathistory labeled-response")
    .server("CAP * ACK :draft/chathistory labeled-response")
    .client("CAP END")
    .welcome(":ergo.test 001 papo :Welcome to Ergo")
    .server(":ergo.test 005 papo CHATHISTORY=100 :are supported")
    .server(":ergo.test 376 papo :End of MOTD")
    .event(|event| matches!(event, Event::Connected(_)))
    .update(|session| {
        session
            .chathistory(
                std::time::Instant::now(),
                HistoryQuery::__latest("#papo".into(), None, 50),
            )
            .unwrap();
        session
            .request_with_matcher(
                std::time::Instant::now(),
                Line::join("#papo"),
                ResponseMatcher::Join("#papo".into()),
            )
            .unwrap();
    })
    .client("@label=0 CHATHISTORY LATEST #papo * 50")
    .client("@label=1 JOIN #papo")
    .server("CAP papo DEL :draft/chathistory")
    .event(|event| matches!(event, Event::CapsChanged { .. }))
    .event(|event| matches!(event, Event::FeaturesChanged { .. }))
    .event(|event| matches!(event, Event::RequestUnsupported { .. }))
    .server("@label=1 :papo!~papo@localhost JOIN #papo * :Papo Bot")
    .event(|event| matches!(event, Event::Joined { channel, nick, .. } if channel == "#papo" && nick == "papo"))
    .no_event()
    .server(":ergo.test 353 papo = #papo :@papo alice")
    .server(":ergo.test 366 papo #papo :End of /NAMES list")
    .client("MODE #papo")
    .client("WHO #papo")
    .event(|event| matches!(event, Event::ChannelSynced { channel, .. } if channel == "#papo"))
    .event(|event| matches!(event, Event::Response(response) if response.lines.len() == 2))
    .no_event();
}

#[test]
fn extension_requests_preserve_explicitly_disabled_reasons() {
    let caps = [
        "draft/metadata-3",
        "draft/account-registration",
        "draft/persistence",
        "draft/webpush",
        "draft/channel-rename",
        "draft/relaymsg",
        "labeled-response",
    ];
    Transcript::new(
        enable_extensions(Config::default())
            .wanted_caps(caps)
            .without_caps(caps),
    )
    .connect_default()
    .server("CAP * LS :draft/metadata-3 draft/account-registration draft/persistence draft/webpush draft/channel-rename draft/relaymsg=/ labeled-response")
    .client("CAP END")
    .update(|session| {
        let now = std::time::Instant::now();
        let expected = |feature| {
            unsupported_because(feature, UnsupportedReason::DisabledByClient)
        };
        assert_eq!(
            session.metadata(now, MetadataCommand::Subs),
            Err(expected(Feature::Metadata))
        );
        assert_eq!(
            session.persistence_get(now),
            Err(expected(Feature::Persistence))
        );
        assert_eq!(
            session.create_account(now, Some("alice"), None, "account-password"),
            Err(expected(Feature::AccountCreation))
        );
        assert_eq!(
            session.verify(now, "alice", "39gvcdg4myvnmdcfhvd6exsv4n"),
            Err(expected(Feature::AccountCreation))
        );
        assert_eq!(
            session.persistence_set(now, PersistenceSetting::On),
            Err(expected(Feature::Persistence))
        );
        assert_eq!(
            session.webpush_register(now, "https://push.example/sub", "AQ", "Ag"),
            Err(expected(Feature::WebPush))
        );
        assert_eq!(
            session.webpush_unregister(now, "https://push.example/sub"),
            Err(expected(Feature::WebPush))
        );
        assert_eq!(
            session.rename(now, "#papo", "#papo-chat", "clearer name"),
            Err(expected(Feature::ChannelRename))
        );
        assert_eq!(
            session.relay(now, "#papo", "matrix/alice", "hello from Matrix"),
            Err(expected(Feature::Relay))
        );
        assert_eq!(
            session.request(now, Line::new("WHOIS", ["alice"])),
            Err(expected(Feature::LabeledResponse))
        );
    })
    .no_client();
}
