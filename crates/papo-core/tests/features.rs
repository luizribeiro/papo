//! Transcript tests for optional feature predicates.

mod support;

use papo_core::{Config, Feature, Session, Unsupported, UnsupportedReason};

const CAPS: &str = "message-tags draft/message-redaction draft/multiline chathistory read-marker draft/metadata-3 setname draft/account-registration draft/persistence draft/webpush draft/channel-rename draft/relaymsg labeled-response";

fn session(caps: &str) -> Session {
    session_with(
        Config::default().wanted_caps(caps.split_whitespace()),
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
        Feature::AccountRegistration,
        Feature::Persistence,
        Feature::WebPush,
        Feature::ChannelRename,
        Feature::Relay,
        Feature::LabeledResponse,
    ] {
        assert_eq!(session.supports(feature), Ok(()), "{feature:?}");
    }
}

#[test]
fn combines_isupport_tokens_with_negotiated_caps() {
    let session = session_with(
        Config::default().wanted_caps(["chathistory"]),
        "chathistory",
        ":irc.unreal.example 005 papo MONITOR=100 BOT=B CHATHISTORY=100 :are supported by this server",
    );
    for feature in [Feature::History, Feature::Monitor, Feature::BotMode] {
        assert_eq!(session.supports(feature), Ok(()), "{feature:?}");
    }
}

#[test]
fn rejects_literal_zero_feature_limits() {
    let zero = session_with(
        Config::default().wanted_caps(["chathistory"]),
        "chathistory",
        ":irc.unreal.example 005 papo MONITOR=0 CHATHISTORY=0 :are supported by this server",
    );
    for feature in [Feature::Monitor, Feature::History] {
        assert_eq!(
            zero.supports(feature),
            Err(papo_core::UnsupportedReason::LimitZero)
        );
    }
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
        Feature::AccountRegistration,
        Feature::Persistence,
        Feature::WebPush,
        Feature::ChannelRename,
        Feature::Relay,
        Feature::LabeledResponse,
    ];
    let disabled = session_with(
        Config::default()
            .wanted_caps(CAPS.split_whitespace())
            .without_caps(CAPS.split_whitespace()),
        CAPS,
        "",
    );
    let absent = session("");
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
            Unsupported {
                feature: Feature::Reactions,
                reason,
            }
            .to_string(),
            format!("reactions unavailable: {expected}")
        );
    }
}
