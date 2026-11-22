use std::fmt;

use crate::{AccountStatus, Casemapping, Nick};

/// The user or server that caused an IRC event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Actor {
    /// A user identity whose mutable facts are available from the event's paired state.
    User(crate::UserHandle),
    /// A server name.
    Server(crate::ServerName),
}

impl Actor {
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
}

impl fmt::Display for Actor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User(user) => user.fmt(formatter),
            Self::Server(server) => server.fmt(formatter),
        }
    }
}

pub(crate) enum LinePrefix<'a> {
    Raw(&'a str),
    Core(papo_core::Actor),
}

pub(crate) fn from_line(
    prefix: LinePrefix<'_>,
    envelope: &super::EventEnvelope,
    casemapping: Casemapping,
    origin: crate::Origin,
) -> Option<Actor> {
    let nick = match prefix {
        LinePrefix::Raw(prefix) => {
            let line = papo_proto::Line::new("TAGMSG")
                .param("*")
                .with_source(prefix);
            match line.as_ref().source()? {
                papo_proto::Source::User { nick, .. } => nick.to_owned(),
                papo_proto::Source::Server(server) => {
                    return Some(Actor::Server(server.into()));
                }
            }
        }
        LinePrefix::Core(source) => match source {
            papo_core::Actor::User { nick, .. } => nick.into_string(),
            papo_core::Actor::Server { name, .. } => {
                return Some(Actor::Server(name));
            }
            _ => return None,
        },
    };
    Some(Actor::User(crate::UserHandle::from_past_line(
        origin.__nick(casemapping, nick),
        envelope.account.clone(),
    )))
}

pub(crate) fn from_core(source: papo_core::Actor, origin: crate::Origin) -> Option<Actor> {
    match source {
        papo_core::Actor::User { nick, .. } => {
            Some(Actor::User(crate::UserHandle::from_past_line(
                origin.__nick(nick.casemapping(), nick.as_str()),
                AccountStatus::Unknown,
            )))
        }
        papo_core::Actor::Server { name, .. } => Some(Actor::Server(name)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use papo_proto::Casemapping;

    #[test]
    fn displays_user_nicks_and_server_names() {
        let mapping = Casemapping::Ascii;
        let full = crate::__testing::source(
            papo_proto::Source::__user("emily", Some("~emily"), Some("localhost")),
            mapping,
        );
        let bare =
            crate::__testing::source(papo_proto::Source::__user("ChanServ", None, None), mapping);
        let server =
            crate::__testing::source(papo_proto::Source::Server("irc.example.net"), mapping);

        assert_eq!(full.user().unwrap().to_string(), "emily");
        assert_eq!(full.to_string(), "emily");
        assert_eq!(bare.user().unwrap().to_string(), "ChanServ");
        assert_eq!(bare.to_string(), "ChanServ");
        assert_eq!(server.to_string(), "irc.example.net");
    }

    #[test]
    fn every_user_named_by_a_line_prefix_is_historical() {
        let mapping = Casemapping::Rfc1459;
        let origin = crate::Origin::NONE;
        let envelope = super::super::EventEnvelope {
            msgid: None,
            server_time: None,
            received_at: std::time::SystemTime::UNIX_EPOCH,
            account: crate::AccountStatus::LoggedIn(origin.__account(mapping, "alice-account")),
        };
        let core = || {
            papo_core::Actor::from_proto(
                papo_proto::Source::__user("alice", Some("~alice"), Some("cloak.example")),
                mapping,
            )
        };
        let sources = [
            super::from_line(
                super::LinePrefix::Raw("alice!~alice@cloak.example"),
                &envelope,
                mapping,
                origin,
            ),
            super::from_line(super::LinePrefix::Core(core()), &envelope, mapping, origin),
            super::from_core(core(), origin),
        ];
        let unknown = crate::AccountStatus::Unknown;
        for (source, account) in
            sources
                .iter()
                .zip([&envelope.account, &envelope.account, &unknown])
        {
            let user = source.as_ref().and_then(super::Actor::user).unwrap();
            assert_eq!(user.nick().as_str(), "alice");
            assert_eq!(user.account(), account, "{source:?}");
            assert!(user.is_unproven_history(), "{source:?}");
        }

        let server = super::from_line(
            super::LinePrefix::Raw("ergo.test"),
            &envelope,
            mapping,
            origin,
        );
        assert_eq!(
            server.map(|source| source.to_string()).as_deref(),
            Some("ergo.test")
        );
    }
}
