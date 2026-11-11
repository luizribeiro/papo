use std::fmt;

use crate::{Account, Casemapping, Nick};

/// The user or server that caused an IRC event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Source {
    /// A user identity whose mutable facts are available from the event's paired state.
    User(crate::UserHandle),
    /// A server name.
    Server(crate::ServerName),
}

impl Source {
    /// Returns the nickname for a user source.
    #[must_use]
    pub fn nick(&self) -> Option<Nick> {
        match self {
            Self::User(user) => Some(user.nick()),
            Self::Server(_) => None,
        }
    }

    /// Returns the user source when the actor is a user.
    #[must_use]
    pub const fn user(&self) -> Option<&crate::UserHandle> {
        match self {
            Self::User(user) => Some(user),
            Self::Server(_) => None,
        }
    }

    /// Builds a source from a parsed protocol prefix without line tags.
    #[doc(hidden)]
    #[must_use]
    pub fn from_proto(source: papo_proto::Source<'_>, casemapping: Casemapping) -> Self {
        from_proto_with_origin(source, casemapping, crate::Origin::NONE)
    }
}

impl fmt::Display for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User(user) => user.fmt(formatter),
            Self::Server(server) => server.fmt(formatter),
        }
    }
}

pub(crate) fn from_proto_with_origin(
    source: papo_proto::Source<'_>,
    casemapping: Casemapping,
    origin: crate::Origin,
) -> Source {
    match source {
        papo_proto::Source::User { nick, .. } => Source::User(crate::UserHandle::from_parts(
            origin.__nick(casemapping, nick),
            Account::Unknown,
            None,
            None,
        )),
        papo_proto::Source::Server(server) => Source::Server(server.into()),
    }
}

pub(crate) enum LinePrefix<'a> {
    Raw(&'a str),
    Core(papo_core::Source),
}

pub(crate) fn from_line(
    prefix: LinePrefix<'_>,
    envelope: &super::EventEnvelope,
    casemapping: Casemapping,
    origin: crate::Origin,
    _prove_not_bot: bool,
) -> Option<Source> {
    let nick = match prefix {
        LinePrefix::Raw(prefix) => {
            let line = papo_proto::Line::new("TAGMSG")
                .param("*")
                .with_source(prefix);
            match line.as_ref().source()? {
                papo_proto::Source::User { nick, .. } => nick.to_owned(),
                papo_proto::Source::Server(server) => {
                    return Some(Source::Server(server.into()));
                }
            }
        }
        LinePrefix::Core(source) => match source {
            papo_core::Source::User { nick, .. } => nick.into_string(),
            papo_core::Source::Server { name, .. } => {
                return Some(Source::Server(name));
            }
            _ => return None,
        },
    };
    Some(Source::User(crate::UserHandle::from_parts(
        origin.__nick(casemapping, nick),
        envelope.account.clone(),
        None,
        None,
    )))
}

pub(crate) fn from_core(source: papo_core::Source, origin: crate::Origin) -> Option<Source> {
    match source {
        papo_core::Source::User { nick, .. } => Some(Source::User(crate::UserHandle::from_parts(
            origin.__nick(nick.casemapping(), nick.as_str()),
            Account::Unknown,
            None,
            None,
        ))),
        papo_core::Source::Server { name, .. } => Some(Source::Server(name)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use papo_proto::Casemapping;

    use super::Source;

    #[test]
    fn displays_user_nicks_and_server_names() {
        let mapping = Casemapping::Ascii;
        let full = Source::from_proto(
            papo_proto::Source::__user("emily", Some("~emily"), Some("localhost")),
            mapping,
        );
        let bare = Source::from_proto(papo_proto::Source::__user("ChanServ", None, None), mapping);
        let server = Source::from_proto(papo_proto::Source::Server("irc.example.net"), mapping);

        assert_eq!(full.user().unwrap().to_string(), "emily");
        assert_eq!(full.to_string(), "emily");
        assert_eq!(bare.user().unwrap().to_string(), "ChanServ");
        assert_eq!(bare.to_string(), "ChanServ");
        assert_eq!(server.to_string(), "irc.example.net");
    }
}
