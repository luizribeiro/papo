use crate::{Command, Isupport, MessageRef, Numeric, source::split_user};

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
    (NameKind::Capability, "draft/chathistory", "chathistory"),
    (NameKind::Isupport, "draft/CHATHISTORY", "CHATHISTORY"),
    (NameKind::Isupport, "soju.im/SAFERATE", "SAFERATE"),
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

/// One member entry from an `RPL_NAMREPLY` (`353`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamesEntry<'a> {
    /// The member's advertised channel-prefix symbols in wire order.
    pub prefixes: Vec<char>,
    /// The member's nickname.
    pub nick: &'a str,
    /// The member's username when `userhost-in-names` is active.
    pub user: Option<&'a str>,
    /// The member's hostname when `userhost-in-names` is active.
    pub host: Option<&'a str>,
}

impl<'a> NamesEntry<'a> {
    /// Parses one NAMES entry using the server's advertised prefix symbols.
    #[must_use]
    pub fn parse(entry: &'a str, isupport: &Isupport) -> Self {
        let advertised = isupport.prefix();
        let prefix_bytes = entry
            .char_indices()
            .take_while(|(_, character)| {
                advertised.iter().any(|prefix| prefix.symbol == *character)
            })
            .map(|(index, character)| index + character.len_utf8())
            .last()
            .unwrap_or(0);
        let (prefixes, source) = entry.split_at(prefix_bytes);
        let (nick, user, host) = split_user(source);
        Self {
            prefixes: prefixes.chars().collect(),
            nick,
            user,
            host,
        }
    }
}

/// Parses an `RPL_NAMREPLY` (`353`) into its type, channel, and raw entries.
#[must_use]
pub fn parse_353<'a>(
    message: &MessageRef<'a>,
) -> Option<(&'a str, &'a str, impl Iterator<Item = &'a str>)> {
    if message.command() != Command::Numeric(Numeric::RPL_NAMREPLY) || message.len() != 4 {
        return None;
    }
    let mut params = message.params();
    params.next()?;
    let channel_type = params.next()?;
    let channel = params.next()?;
    let entries = params.next()?.split_whitespace();
    Some((channel_type, channel, entries))
}
