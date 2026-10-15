//! Integration tests for ratified protocol-name aliases.

use papo_proto::{Advertised, Isupport, NAME_ALIASES, NameKind, NamesEntry, canonical, parse_353};

#[test]
fn maps_every_draft_name_and_preserves_every_ratified_name() {
    let expected = [
        (NameKind::Tag, "+draft/reply", "+reply"),
        (NameKind::Tag, "+draft/channel-context", "+channel-context"),
        (
            NameKind::Capability,
            "draft/no-implicit-names",
            "no-implicit-names",
        ),
        (NameKind::Capability, "draft/chathistory", "chathistory"),
        (NameKind::Isupport, "draft/CHATHISTORY", "CHATHISTORY"),
    ];

    assert_eq!(NAME_ALIASES, expected);
    for (_, draft, ratified) in expected {
        assert_eq!(canonical(draft), ratified);
        assert_eq!(canonical(ratified), ratified);
    }
}

#[test]
fn applies_case_rules_for_each_name_kind() {
    assert_eq!(canonical("DRAFT/CHATHISTORY"), "CHATHISTORY");
    assert_eq!(canonical("chathistory"), "chathistory");
    assert_eq!(canonical("+DRAFT/REPLY"), "+DRAFT/REPLY");
    assert_eq!(
        canonical("DRAFT/NO-IMPLICIT-NAMES"),
        "DRAFT/NO-IMPLICIT-NAMES"
    );
}

#[test]
fn returns_unknown_names_unchanged() {
    let name = String::from("draft/future-extension");
    assert!(std::ptr::eq(canonical(&name), name.as_str()));
}

#[test]
fn isupport_normalizes_draft_history_names_case_insensitively() {
    let mut support = Isupport::default();
    support.apply_tokens(["DRAFT/CHATHISTORY=1000"]);

    assert_eq!(support.get("CHATHISTORY"), Advertised::Value("1000"));
    assert_eq!(support.get("DRAFT/CHATHISTORY"), Advertised::Value("1000"));

    support.apply_tokens(["-DRAFT/CHATHISTORY"]);
    assert_eq!(support.get("CHATHISTORY"), Advertised::Absent);
}

#[test]
fn parses_multi_prefix_and_userhost_entries_together_and_apart() {
    let mut support = Isupport::default();
    support.apply_tokens(["PREFIX=(qaohv)~&@%+"]);

    assert_eq!(
        NamesEntry::parse("@+alice!~alice@user/alice", &support),
        entry(&['@', '+'], "alice", Some("~alice"), Some("user/alice"))
    );
    assert_eq!(
        NamesEntry::parse("bob!~bob@192.0.2.4", &support),
        entry(&[], "bob", Some("~bob"), Some("192.0.2.4"))
    );
    assert_eq!(
        NamesEntry::parse("%carol", &support),
        entry(&['%'], "carol", None, None)
    );
    assert_eq!(
        NamesEntry::parse("dan", &support),
        entry(&[], "dan", None, None)
    );
}

#[test]
fn strips_only_advertised_prefix_characters() {
    let mut support = Isupport::default();
    support.apply_tokens(["PREFIX=(ov)@+"]);

    assert_eq!(
        NamesEntry::parse("~alice!ident@cloak.example", &support),
        entry(&[], "~alice", Some("ident"), Some("cloak.example"))
    );
    assert_eq!(NamesEntry::parse("", &support), entry(&[], "", None, None));
}

#[test]
fn parses_realistic_names_reply_and_rejects_wrong_shapes() {
    let message = papo_proto::MessageRef::parse(
        ":irc.example 353 tester = #papo :@+alice!~alice@user/alice bob!~bob@192.0.2.4",
    )
    .unwrap();
    let (channel_type, channel, entries) = parse_353(&message).unwrap();
    assert_eq!(channel_type, "=");
    assert_eq!(channel, "#papo");
    assert_eq!(
        entries.collect::<Vec<_>>(),
        ["@+alice!~alice@user/alice", "bob!~bob@192.0.2.4"]
    );

    for line in [
        ":irc.example 352 tester #papo ~alice host server alice H :0 Alice",
        ":irc.example 353 tester = #papo",
        ":irc.example 353 tester = #papo alice extra",
    ] {
        assert!(parse_353(&papo_proto::MessageRef::parse(line).unwrap()).is_none());
    }
}

fn entry<'a>(
    prefixes: &[char],
    nick: &'a str,
    user: Option<&'a str>,
    host: Option<&'a str>,
) -> NamesEntry<'a> {
    NamesEntry {
        prefixes: prefixes.to_vec(),
        nick,
        user,
        host,
    }
}
