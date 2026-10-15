//! Integration tests for ratified protocol-name aliases.

use papo_proto::{Advertised, Isupport, NAME_ALIASES, NameKind, canonical};

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
    assert_eq!(canonical("chathistory"), "CHATHISTORY");
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
