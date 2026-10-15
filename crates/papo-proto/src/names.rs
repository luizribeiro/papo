/// The namespace containing a protocol name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameKind {
    /// An `IRCv3` capability name.
    Capability,
    /// An `IRCv3` message-tag name.
    Tag,
    /// An ISUPPORT token name.
    Isupport,
}

/// Draft protocol names and the names that replaced them at ratification.
pub const NAME_ALIASES: &[(NameKind, &str, &str)] = &[
    (NameKind::Tag, "+draft/reply", "+reply"),
    (NameKind::Tag, "+draft/channel-context", "+channel-context"),
    (
        NameKind::Capability,
        "draft/no-implicit-names",
        "no-implicit-names",
    ),
    (NameKind::Isupport, "draft/CHATHISTORY", "CHATHISTORY"),
];

/// Returns the ratified spelling of a protocol name when one is known.
///
/// Capability and message-tag names are matched case-sensitively. ISUPPORT
/// names are matched ASCII case-insensitively. Unknown names are returned unchanged.
#[must_use]
pub fn canonical(name: &str) -> &str {
    NAME_ALIASES
        .iter()
        .find_map(|&(kind, draft, ratified)| {
            let matches = match kind {
                NameKind::Capability | NameKind::Tag => name == draft || name == ratified,
                NameKind::Isupport => {
                    name.eq_ignore_ascii_case(draft) || name.eq_ignore_ascii_case(ratified)
                }
            };
            matches.then_some(ratified)
        })
        .unwrap_or(name)
}
