/// The source of an IRC event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Source {
    /// A user prefix.
    #[non_exhaustive]
    User {
        /// The user's nickname.
        nick: String,
        /// The username, when supplied by the server.
        user: Option<String>,
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

impl Source {
    /// Returns the nickname for a user source.
    #[must_use]
    pub fn nick(&self) -> Option<&str> {
        match self {
            Self::User { nick, .. } => Some(nick),
            Self::Server { .. } => None,
        }
    }

    pub(crate) fn from_proto(source: papo_proto::Source<'_>) -> Self {
        match source {
            papo_proto::Source::Server(name) => Self::Server {
                name: name.to_owned(),
            },
            papo_proto::Source::User { nick, user, host } => Self::User {
                nick: nick.to_owned(),
                user: user.map(str::to_owned),
                host: host.map(str::to_owned),
            },
        }
    }

    pub(crate) fn from_prefix(prefix: String) -> Self {
        let message = papo_proto::Message::new("NOTICE", ["*"]).with_source(prefix);
        Self::from_proto(message.as_ref().source().expect("owned source is present"))
    }
}
