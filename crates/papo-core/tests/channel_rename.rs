//! Transcript tests for `draft/channel-rename`.

mod support;

use std::time::Duration;

use papo_core::{Config, Conversation, Event, Feature, Refusal, TypingReason};
use support::{Transcript, unsupported_error};

fn registered() -> Transcript {
    let caps = "draft/channel-rename labeled-response message-tags server-time draft/read-marker";
    Transcript::new(Config::default().wanted_caps(caps.split_whitespace()))
        .connect_default()
        .server(&format!("CAP * LS :{caps}"))
        .client(&format!("CAP REQ :{caps}"))
        .server(&format!("CAP * ACK :{caps}"))
        .client("CAP END")
        .welcome(":ergo.test 001 alice :Welcome to Ergo")
        .server(":ergo.test 422 alice :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
}

#[test]
fn sends_rename_and_types_failures() {
    for (code, expected) in [
        ("CHANNEL_NAME_IN_USE", Refusal::ChannelNameInUse),
        ("CANNOT_RENAME", Refusal::ChannelRenameForbidden),
    ] {
        registered()
            .update(|session| {
                session
                    .rename(
                        std::time::Instant::now(),
                        "#rust",
                        "#rust-lang",
                        "Project renamed",
                    )
                    .unwrap();
            })
            .client("@label=0 RENAME #rust #rust-lang :Project renamed")
            .server(&format!(
                "@label=0 :ergo.test FAIL RENAME {code} :Channel rename failed"
            ))
            .event(|event| {
                matches!(event, Event::Response(response)
                if response.refusal() == Some(expected))
            });
    }
}

#[test]
fn omits_an_empty_rename_reason() {
    registered()
        .update(|session| {
            session
                .rename(std::time::Instant::now(), "#rust", "#rust-lang", "")
                .unwrap();
        })
        .client("@label=0 RENAME #rust #rust-lang");
}

#[test]
fn types_an_unsolicited_rename_failure() {
    registered()
        .server(":ergo.test FAIL RENAME CANNOT_RENAME :Persistent channels cannot be renamed")
        .event(|event| {
            matches!(event, Event::StandardReply(reply)
            if reply.refusal() == Some(Refusal::ChannelRenameForbidden))
        });
}

#[test]
fn requires_the_channel_rename_capability() {
    Transcript::new(Config::default())
        .update(|session| {
            assert_eq!(
                session.rename(std::time::Instant::now(), "#rust", "#Rust", ""),
                Err(unsupported_error(Feature::ChannelRename))
            );
        })
        .no_client();
}

#[test]
fn rekeys_channel_members_and_metadata() {
    registered()
        .server(":ergo.test 005 papo CASEMAPPING=ascii :are supported")
        .event(|event| matches!(event, Event::CasemappingChanged { .. }))
        .server(":alice!a@localhost JOIN #rust * :Alice Example")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":bob!b@localhost JOIN #rust * :Bob Example")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":bob!b@localhost METADATA #rust status * :shipping")
        .event(|event| matches!(event, Event::MetadataChanged { .. }))
        .server(":alice!a@localhost RENAME #rust #rust-lang :Project renamed")
        .event(|event| {
            matches!(event, Event::ChannelRenamed { old, new, actor, reason, .. }
            if old == "#rust" && new == "#rust-lang"
                && old.casemapping() == papo_proto::Casemapping::Ascii
                && new.casemapping() == papo_proto::Casemapping::Ascii
                && actor == "alice!a@localhost"
                && reason.as_deref() == Some("Project renamed"))
        })
        .inspect(|session| {
            assert!(session.state().channel("#rust").is_none());
            assert!(
                session
                    .state()
                    .channel("#rust-lang")
                    .unwrap()
                    .member("bob")
                    .is_some()
            );
            assert_eq!(
                session
                    .state()
                    .user("bob")
                    .unwrap()
                    .channels()
                    .collect::<Vec<_>>(),
                ["#rust-lang"]
            );
            let metadata = session.state().metadata("#rust-lang").unwrap();
            assert_eq!(metadata.get("status").unwrap().value, "shipping");
            assert!(session.state().metadata("#rust").is_none());
        });
}

#[test]
fn applies_case_only_channel_renames() {
    registered()
        .server(":alice!a@localhost JOIN #Rust * :Alice Example")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":alice!a@localhost RENAME #Rust #rust :")
        .event(|event| matches!(event, Event::ChannelRenamed { reason: None, .. }))
        .inspect(|session| assert_eq!(session.state().channel("#RUST").unwrap().name(), "#rust"));
}

#[test]
fn accepts_a_rename_without_a_reason() {
    registered()
        .server(":alice!a@localhost JOIN #rust * :Alice Example")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":ergo.test RENAME #rust #rust-lang")
        .event(|event| {
            matches!(event, Event::ChannelRenamed { actor, reason: None, .. }
                if actor == "ergo.test")
        });
}

#[test]
fn rename_moves_every_channel_keyed_state_store() {
    registered()
        .server(":alice!a@localhost JOIN #rust * :Alice Example")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":bob!b@localhost JOIN #rust * :Bob Example")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server("@+typing=active :bob!b@localhost TAGMSG #rust")
        .event(|event| matches!(event, Event::TypingChanged(_)))
        .server("MARKREAD #rust timestamp=2026-10-27T12:00:00.000Z")
        .event(|event| matches!(event, Event::ReadMarkerChanged(_)))
        .server(
            "@msgid=RfcBTzyxY120004;time=2026-10-27T12:00:01.000Z :carol!c@localhost PRIVMSG #rust :still here",
        )
        .event(|event| matches!(event, Event::Line(_)))
        .server(":alice!a@localhost RENAME #rust #rust-lang :Project renamed")
        .event(|event| matches!(event, Event::ChannelRenamed { .. }))
        .inspect(|session| {
            let state = session.state();
            let old = state.conversation("#rust").unwrap();
            let new = state.conversation("#rust-lang").unwrap();
            assert_eq!(state.typing(&old).count(), 0);
            let typer = state.typing(&new).next().unwrap();
            assert_eq!(typer.return_address(), "#rust-lang");
            assert!(state.read_marker("#rust").is_none());
            let marker = state.read_marker("#rust-lang").unwrap();
            assert!(matches!(
                &marker.conversation,
                Conversation::Channel(channel) if channel == "#rust-lang"
            ));
            assert!(state.last_seen("#rust").is_none());
            let seen = state.last_seen("#rust-lang").unwrap();
            assert_eq!(seen.target, "#rust-lang");
        })
        .advance(Duration::from_secs(6))
        .event(|event| {
            matches!(event, Event::TypingChanged(change)
                if matches!(&change.conversation, Conversation::Channel(channel) if channel == "#rust-lang")
                    && change.reason == TypingReason::TimedOut)
        });
}
