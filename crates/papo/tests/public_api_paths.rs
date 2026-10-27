//! Checks that facade types have one canonical public path.

use std::collections::{BTreeMap, BTreeSet};

const STABLE: &str = include_str!("../public-api.txt");
const UNSTABLE: &str = include_str!("../public-api-unstable.txt");
const BOT: &str = include_str!("../../papo-bot/public-api.txt");
const CORE: &str = include_str!("../../papo-core/public-api.txt");
const CORE_UNSTABLE: &str = include_str!("../../papo-core/public-api-unstable.txt");
const PROTO: &str = include_str!("../../papo-proto/public-api.txt");
const REEXPORTED_SURFACES: &str = "# re-exported type surfaces";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConversationClass {
    Event,
    History,
    Outgoing,
    State,
}

const PAPO_CONVERSATION_ITEMS: &[(&str, ConversationClass)] = &[
    (
        "pub papo::event::CtcpQuery::conversation: papo::Conversation",
        ConversationClass::Event,
    ),
    (
        "pub papo::event::CtcpReply::conversation: papo::Conversation",
        ConversationClass::Event,
    ),
    (
        "pub papo::event::Notice::conversation: papo::Conversation",
        ConversationClass::Event,
    ),
    (
        "pub papo::event::Reacted::conversation: papo::Conversation",
        ConversationClass::Event,
    ),
    (
        "pub papo::event::ReadMarkerChanged::conversation: papo::Conversation",
        ConversationClass::Event,
    ),
    (
        "pub papo::event::Redacted::conversation: papo::Conversation",
        ConversationClass::Event,
    ),
    (
        "pub papo::event::TagMsg::conversation: papo::Conversation",
        ConversationClass::Event,
    ),
    (
        "pub papo::event::TypingChanged::conversation: papo::Conversation",
        ConversationClass::Event,
    ),
    (
        "pub fn papo::Event::conversation(&self) -> core::option::Option<papo::Conversation>",
        ConversationClass::Event,
    ),
    (
        "pub papo::HistoryPage::conversation: papo::Conversation",
        ConversationClass::History,
    ),
    (
        "pub papo::Message::conversation: papo::Conversation",
        ConversationClass::Event,
    ),
    (
        "pub fn papo::NameRules<'_>::conversation(&self, &str) -> core::result::Result<papo::Conversation, papo::InvalidInput>",
        ConversationClass::State,
    ),
    (
        "pub fn papo::NameRules<'_>::target(&self, &str) -> core::result::Result<(core::option::Option<papo::Prefix>, papo::Conversation), papo::InvalidInput>",
        ConversationClass::State,
    ),
    (
        "pub papo::RecentConversation::conversation: papo::Conversation",
        ConversationClass::History,
    ),
    (
        "pub papo::Sent::conversation: papo::Conversation",
        ConversationClass::Outgoing,
    ),
    (
        "impl papo::AsConversation for papo::Conversation",
        ConversationClass::State,
    ),
];
const BOT_CONVERSATION_ITEMS: &[(&str, ConversationClass)] = &[];

fn public_type_path(line: &str) -> Option<&str> {
    let public = line.find("pub ")?;
    let declaration = &line[public + 4..];
    let path = if let Some(path) = declaration.strip_prefix("use ") {
        path
    } else {
        ["struct ", "enum ", "union ", "trait ", "type "]
            .into_iter()
            .find_map(|kind| declaration.strip_prefix(kind))?
    };
    let path = path
        .split(|character: char| !(character.is_ascii_alphanumeric() || "_:".contains(character)))
        .next()?
        .trim_end_matches(':');
    let segments: Vec<_> = path.split("::").collect();
    let name = *segments.last()?;
    if !name.chars().next().is_some_and(char::is_uppercase)
        || segments[1..segments.len() - 1]
            .iter()
            .any(|segment| segment.starts_with(char::is_uppercase))
    {
        return None;
    }
    Some(path)
}

fn duplicate_type_paths(snapshot: &str) -> Vec<Vec<&str>> {
    let mut paths = BTreeMap::<_, BTreeSet<_>>::new();
    for path in facade_items(snapshot).lines().filter_map(public_type_path) {
        let name = path.rsplit("::").next().expect("a public path has a name");
        paths.entry(name).or_default().insert(path);
    }
    paths
        .into_values()
        .filter(|paths| paths.len() > 1)
        .map(|paths| paths.into_iter().collect())
        .collect()
}

fn unsealed_struct_variants(snapshot: &str) -> Vec<&str> {
    let lines: Vec<_> = snapshot.lines().collect();
    lines
        .windows(2)
        .filter_map(|pair| {
            let variant = pair[0].strip_prefix("pub ")?;
            let field = pair[1].strip_prefix("pub ")?.strip_prefix(variant)?;
            let field = field.strip_prefix("::")?.split_once(':')?.0;
            (variant.matches("::").count() >= 2
                && variant.rsplit("::").next()?.starts_with(char::is_uppercase)
                && field.starts_with(char::is_lowercase))
            .then_some(variant)
        })
        .collect()
}

fn wire_address_exposures(snapshot: &str) -> Vec<&str> {
    facade_items(snapshot)
        .lines()
        .filter(|line| {
            line.split(|character: char| {
                !(character.is_ascii_alphanumeric() || "_:".contains(character))
            })
            .any(|word| word == "papo_proto::Address")
        })
        .collect()
}

fn internal_crate_paths(snapshot: &str) -> Vec<&str> {
    facade_items(snapshot)
        .lines()
        .filter(|line| line.contains("papo_core::") || line.contains("papo_proto::"))
        .collect()
}

fn conversation_items(snapshot: &str) -> Vec<&str> {
    facade_items(snapshot)
        .lines()
        .filter(|line| *line != "pub use papo::Conversation" && line.contains("papo::Conversation"))
        .collect()
}

fn facade_items(snapshot: &str) -> &str {
    snapshot
        .split_once(REEXPORTED_SURFACES)
        .map_or(snapshot, |(facade, _surfaces)| facade)
}

#[test]
fn public_types_have_one_path() {
    for (name, snapshot) in [
        ("papo", STABLE),
        ("papo unstable", UNSTABLE),
        ("papo-bot", BOT),
    ] {
        assert_eq!(
            duplicate_type_paths(snapshot),
            Vec::<Vec<&str>>::new(),
            "{name} exposes a type through more than one public path"
        );
    }
}

#[test]
fn path_check_finds_root_and_module_reexports() {
    let snapshot = "pub use example::Thing\n#[non_exhaustive] pub struct example::event::Thing\n";
    assert_eq!(
        duplicate_type_paths(snapshot),
        vec![vec!["example::Thing", "example::event::Thing"]]
    );
}

#[test]
fn struct_like_variants_are_non_exhaustive() {
    for (name, snapshot) in [
        ("papo", STABLE),
        ("papo unstable", UNSTABLE),
        ("papo-bot", BOT),
        ("papo-core", CORE),
        ("papo-core unstable", CORE_UNSTABLE),
        ("papo-proto", PROTO),
    ] {
        assert_eq!(
            unsealed_struct_variants(snapshot),
            Vec::<&str>::new(),
            "{name} has struct-like variants without #[non_exhaustive]"
        );
    }
}

#[test]
fn variant_check_distinguishes_struct_and_tuple_variants() {
    let snapshot = "pub example::Event::Changed\npub example::Event::Changed::value: u8\n\
                    #[non_exhaustive] pub example::Event::Ready\npub example::Event::Ready::value: u8\n\
                    pub example::Event::Message(alloc::string::String)\n";
    assert_eq!(
        unsealed_struct_variants(snapshot),
        ["example::Event::Changed"]
    );
}

#[test]
fn public_items_do_not_expose_wire_addresses() {
    for (name, snapshot) in [
        ("papo", STABLE),
        ("papo unstable", UNSTABLE),
        ("papo-bot", BOT),
    ] {
        assert_eq!(
            wire_address_exposures(snapshot),
            Vec::<&str>::new(),
            "{name} exposes papo-proto's wire address type"
        );
    }
}

#[test]
fn address_check_matches_types_not_names() {
    let snapshot = "pub fn example::send(papo_proto::Address)\n\
                    pub example::InvalidInput::Url(papo::InvalidUrl)\n";
    assert_eq!(
        wire_address_exposures(snapshot),
        ["pub fn example::send(papo_proto::Address)"]
    );
}

#[test]
fn public_api_uses_only_papo_paths() {
    for (name, snapshot) in [("papo", STABLE), ("papo unstable", UNSTABLE)] {
        assert_eq!(
            internal_crate_paths(snapshot),
            Vec::<&str>::new(),
            "{name} exposes an internal crate path"
        );
    }
}

#[test]
fn public_api_uses_canonical_message_name() {
    for (name, snapshot) in [
        ("papo", STABLE),
        ("papo unstable", UNSTABLE),
        ("papo-bot", BOT),
        ("papo-core", CORE),
        ("papo-core unstable", CORE_UNSTABLE),
        ("papo-proto", PROTO),
    ] {
        assert!(
            !snapshot.contains("ChatMessage"),
            "{name} exposes the internal ChatMessage name"
        );
    }
}

fn missing_reexported_surfaces(snapshot: &str) -> Vec<&str> {
    let (facade, surfaces) = snapshot
        .split_once(REEXPORTED_SURFACES)
        .expect("the facade snapshot has re-exported surfaces");
    let definitions: BTreeSet<_> = surfaces.lines().filter_map(public_type_path).collect();
    facade
        .lines()
        .filter_map(|line| line.strip_prefix("pub use "))
        .filter(|path| {
            path.rsplit("::")
                .next()
                .is_some_and(|name| name.starts_with(char::is_uppercase))
        })
        .filter(|path| !definitions.contains(path))
        .collect()
}

#[test]
fn reexported_type_surfaces_are_snapshotted() {
    for (name, snapshot) in [("papo", STABLE), ("papo unstable", UNSTABLE)] {
        assert_eq!(
            missing_reexported_surfaces(snapshot),
            Vec::<&str>::new(),
            "{name} omits a re-exported type's surface"
        );
        assert!(
            snapshot.contains("pub papo::ModeChange::mode: char"),
            "{name} omits ModeChange's re-exported fields"
        );
    }
}

#[test]
fn internal_path_check_finds_core_and_proto_paths() {
    let snapshot = "pub use papo::Nick\npub fn papo::core(papo_core::State)\n\
                    pub fn papo::proto(papo_proto::Line)\n";
    assert_eq!(
        internal_crate_paths(snapshot),
        [
            "pub fn papo::core(papo_core::State)",
            "pub fn papo::proto(papo_proto::Line)",
        ]
    );
}

#[test]
fn conversation_items_are_classified() {
    for (name, snapshot, classified) in [
        ("papo", STABLE, PAPO_CONVERSATION_ITEMS),
        ("papo unstable", UNSTABLE, PAPO_CONVERSATION_ITEMS),
        ("papo-bot", BOT, BOT_CONVERSATION_ITEMS),
    ] {
        let classified: Vec<_> = classified.iter().map(|(item, _class)| *item).collect();
        assert_eq!(
            conversation_items(snapshot),
            classified,
            "{name} has an unclassified Conversation-bearing item"
        );
    }
}

#[test]
fn conversation_check_ignores_role_inputs_and_the_reexport() {
    let snapshot = "pub use papo::Conversation\n\
                    pub example::Event::conversation: papo::Conversation\n\
                    pub fn example::history(impl papo::AsConversation)\n";
    assert_eq!(
        conversation_items(snapshot),
        ["pub example::Event::conversation: papo::Conversation"]
    );
}
