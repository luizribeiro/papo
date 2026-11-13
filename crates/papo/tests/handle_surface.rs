//! Checks that the public surface names people and channels only through handles.

const STABLE: &str = include_str!("../public-api.txt");
const UNSTABLE: &str = include_str!("../public-api-unstable.txt");

const RETIRED_IDENTITY_ITEMS: &[&str] = &[
    "papo::DirectRef",
    "papo::DirectSource",
    "papo::PeerPinMismatch",
    "papo::Client::direct(",
    "papo::Client::direct_pinned(",
    "papo::Client::direct_to(",
];

const IDENTITY_FIELDS: &[&str] = &[
    "actor",
    "channel",
    "channel_context",
    "channels",
    "conversation",
    "subject",
    "user",
];

const IDENTITY_TYPES: &[&str] = &[
    "papo::ChannelHandle",
    "papo::Conversation",
    "papo::Source",
    "papo::Subject",
    "papo::UserHandle",
    "alloc::vec::Vec<papo::ChannelHandle>",
    "core::option::Option<papo::ChannelHandle>",
];

const NAME_FACTS: &[&str] = &[
    "papo::Message::relayed_by",
    "papo::event::ChannelRenamed::new",
    "papo::event::ChannelRenamed::old",
    "papo::event::Connected::nick",
    "papo::event::NickChanged::new",
    "papo::event::NickChanged::old",
    "papo::event::Notice::relayed_by",
    "papo::event::Warning::MonitorListFull::nicks",
];

fn apis() -> [(&'static str, &'static str); 2] {
    [("stable", STABLE), ("unstable", UNSTABLE)]
}

fn event_fields(api: &str) -> impl Iterator<Item = (&str, &str)> {
    api.lines()
        .filter_map(|line| line.strip_prefix("pub "))
        .filter(|item| {
            ["papo::event::", "papo::Message::", "papo::WhoEntry::"]
                .iter()
                .any(|prefix| item.starts_with(prefix))
        })
        .filter_map(|item| item.split_once(": "))
}

#[test]
fn direct_ref_api_is_absent() {
    for (name, api) in apis() {
        for item in RETIRED_IDENTITY_ITEMS {
            assert!(!api.contains(item), "the {name} API still exposes {item}");
        }
    }
}

#[test]
fn all_event_person_and_channel_fields_are_handles() {
    for (name, api) in apis() {
        let mut identity_fields = 0;
        for (path, ty) in event_fields(api) {
            let field = path.rsplit("::").next().unwrap();
            if IDENTITY_FIELDS.contains(&field) {
                identity_fields += 1;
                assert!(
                    IDENTITY_TYPES.contains(&ty),
                    "{name} {path} carries {ty} instead of a handle"
                );
            }
            if ty.contains("papo::Nick") || ty.contains("papo::ChannelName") {
                assert!(
                    NAME_FACTS.contains(&path),
                    "{name} {path} carries a bare name {ty}"
                );
            }
        }
        assert!(
            identity_fields >= 40,
            "{name} lists {identity_fields} identity fields"
        );
    }
}
