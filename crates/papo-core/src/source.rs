use std::fmt;

use papo_proto::{Nick, ServerName};

/// The source of an IRC event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Source {
    /// A user prefix.
    #[non_exhaustive]
    User {
        /// The user's nickname.
        nick: Nick,
        /// The username from `nick!user@host`, when supplied by the server.
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
}

impl fmt::Display for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User { nick, .. } => nick.fmt(formatter),
            Self::Server { name } => name.fmt(formatter),
        }
    }
}

#[cfg(test)]
mod tests {
    use papo_proto::{Casemapping, ServerName};

    use super::Source;

    #[test]
    fn returns_only_user_nicknames() {
        let nick = Casemapping::Rfc1459.nick("Alice[bot]");
        let user = Source::User {
            nick: nick.clone(),
            username: Some("~alice".into()),
            host: Some("staff.example".into()),
        };
        let server = Source::Server {
            name: ServerName::from("irc.example.net"),
        };

        assert_eq!(user.nick(), Some(&nick));
        assert_eq!(server.nick(), None);
    }
}
