//! Transcript tests for `draft/channel-rename`.

mod support;

use papo_core::{Config, Event, Feature, Refusal};
use support::{Transcript, unsupported_error};

fn registered() -> Transcript {
    Transcript::new(Config::default().wanted_caps(["draft/channel-rename", "labeled-response"]))
        .connect_default()
        .server("CAP * LS :draft/channel-rename labeled-response")
        .client("CAP REQ :draft/channel-rename labeled-response")
        .server("CAP * ACK :draft/channel-rename labeled-response")
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
