use crate::{Casemapping, Nick};

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
        name: String,
    },
}

impl std::fmt::Display for Source {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User { nick, .. } => formatter.write_str(nick),
            Self::Server { name } => formatter.write_str(name),
        }
    }
}

impl Source {
    /// Returns the nickname for a user source.
    #[must_use]
    pub fn nick(&self) -> Option<&str> {
        match self {
            Self::User { nick, .. } => Some(nick),
            Self::Server { .. } => None,
        }
    }

    pub(crate) fn from_proto(source: papo_proto::Source<'_>, casemapping: Casemapping) -> Self {
        match source {
            papo_proto::Source::Server(name) => Self::Server {
                name: name.to_owned(),
            },
            papo_proto::Source::User {
                nick,
                username,
                host,
            } => Self::User {
                nick: casemapping.nick(nick),
                username: username.map(str::to_owned),
                host: host.map(str::to_owned),
            },
        }
    }

    pub(crate) fn from_prefix(prefix: String, casemapping: Casemapping) -> Self {
        let message = papo_proto::Line::new("NOTICE", ["*"]).with_source(prefix);
        Self::from_proto(
            message.as_ref().source().expect("owned source is present"),
            casemapping,
        )
    }

    pub(crate) fn from_name(name: String, casemapping: Casemapping) -> Self {
        if name.contains('.') {
            Self::Server { name }
        } else {
            Self::User {
                nick: casemapping.nick(name),
                username: None,
                host: None,
            }
        }
    }
}
