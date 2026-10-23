use std::fmt;

use papo_proto::{Casemapping, MaskSource, Nick, ServerName};

use crate::Names;

/// The source of an IRC event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Source {
    /// A user prefix.
    #[non_exhaustive]
    User {
        /// The user's nickname.
        nick: Nick,
        /// The username from `nick!user@host`, when supplied by the server; it may start with `~`.
        username: Option<String>,
        /// The hostname, when supplied by the server.
        host: Option<String>,
    },
    /// A server prefix.
    #[non_exhaustive]
    Server {
        /// The server name.
        name: ServerName,
    },
}

impl Source {
    /// Returns the nickname for a user source.
    #[must_use]
    pub const fn nick(&self) -> Option<&Nick> {
        match self {
            Self::User { nick, .. } => Some(nick),
            Self::Server { .. } => None,
        }
    }

    /// Builds an owned source from a parsed protocol prefix.
    #[doc(hidden)]
    #[must_use]
    pub fn from_proto(source: papo_proto::Source<'_>, casemapping: Casemapping) -> Self {
        Self::from_proto_with_names(source, Names::new(casemapping, papo_proto::Origin::NONE))
    }

    pub(crate) fn from_proto_with_names(source: papo_proto::Source<'_>, names: Names) -> Self {
        match source {
            papo_proto::Source::Server(name) => Self::Server { name: name.into() },
            papo_proto::Source::User {
                nick,
                username,
                host,
                ..
            } => Self::User {
                nick: names.nick(nick),
                username: username.map(str::to_owned),
                host: host.map(str::to_owned),
            },
        }
    }

    pub(crate) fn from_prefix_with_names(prefix: &str, names: Names) -> Self {
        let line = papo_proto::Line::new("NOTICE")
            .param("*")
            .with_source(prefix);
        Self::from_proto_with_names(
            line.as_ref().source().expect("owned source is present"),
            names,
        )
    }
}

impl fmt::Display for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User { nick, .. } => nick.fmt(formatter),
            Self::Server { name } => name.fmt(formatter),
        }
    }
}

impl MaskSource for Source {
    fn mask_parts(&self) -> Option<(&Nick, &str, &str)> {
        match self {
            Self::User {
                nick,
                username: Some(username),
                host: Some(host),
            } => Some((nick, username, host)),
            Self::User { .. } | Self::Server { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use papo_proto::Casemapping;

    use super::Source;

    fn source(prefix: &str, mapping: Casemapping) -> Source {
        Source::from_prefix_with_names(prefix, crate::Names::new(mapping, papo_proto::Origin::NONE))
    }

    #[test]
    fn returns_only_user_nicknames() {
        let nick = Casemapping::Rfc1459.nick("Alice[bot]");
        let user = Source::from_proto(
            papo_proto::Source::__user("Alice[bot]", Some("~alice"), Some("staff.example")),
            Casemapping::Rfc1459,
        );
        let server = Source::from_proto(
            papo_proto::Source::Server("irc.example.net"),
            Casemapping::Rfc1459,
        );

        assert_eq!(user.nick(), Some(&nick));
        assert_eq!(server.nick(), None);
        assert!(matches!(server, Source::Server { name } if name == "IRC.EXAMPLE.NET"));
    }

    #[test]
    fn displays_user_nicks_and_server_names() {
        let mapping = Casemapping::Rfc1459;
        let full_user = source("alice!~alice@staff.example", mapping);
        let bare_user = source("Alice[bot]", mapping);
        let server = source("Irc.Libera.Chat", mapping);

        assert_eq!(full_user.to_string(), "alice");
        assert_eq!(bare_user.to_string(), "Alice[bot]");
        assert_eq!(server.to_string(), "Irc.Libera.Chat");
    }

    #[test]
    fn builds_masks_from_complete_user_sources() {
        let mapping = Casemapping::Rfc1459;
        let source = source("Alice[bot]!~alice@staff.example", mapping);
        let nick = source.nick().unwrap();

        assert_eq!(
            papo_proto::Mask::exact(&source).unwrap(),
            "alice{bot}!~alice@staff.example"
        );
        assert_eq!(
            papo_proto::Mask::for_host(&source).unwrap(),
            "*!*@staff.example"
        );
        assert_eq!(
            papo_proto::Mask::for_user_host(&source).unwrap(),
            "*!~alice@staff.example"
        );
        assert_eq!(papo_proto::Mask::for_nick(nick), "alice{bot}!*@*");
    }

    #[test]
    fn source_mask_builders_require_a_complete_user_prefix() {
        let mapping = Casemapping::Ascii;
        for source in [
            source("Alice!~alice", mapping),
            source("Alice@staff.example", mapping),
            source("irc.example.net", mapping),
        ] {
            assert_eq!(papo_proto::Mask::exact(&source), None);
            assert_eq!(papo_proto::Mask::for_host(&source), None);
            assert_eq!(papo_proto::Mask::for_user_host(&source), None);
        }
    }
}
