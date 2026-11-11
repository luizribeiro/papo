use std::{fmt, sync::Arc};

use crate::{Account, Casemapping, Nick, Oper};

/// A user prefix and the identity facts carried by the same IRC line.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct UserSource {
    nick: Nick,
    username: Option<String>,
    host: Option<String>,
    account: Account,
    peer_key: Option<Arc<crate::peer_key::PeerKey>>,
    is_bot: Option<bool>,
    oper: Option<Oper>,
}

impl UserSource {
    /// Returns the user's nickname.
    #[must_use]
    pub const fn nick(&self) -> &Nick {
        &self.nick
    }

    /// Returns the username supplied in the user prefix.
    #[must_use]
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// Returns the hostname supplied in the user prefix.
    #[must_use]
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// Returns the account state reported by this line.
    #[must_use]
    pub const fn account(&self) -> &Account {
        &self.account
    }

    /// Returns the identity pin when this source sent a private-conversation event.
    ///
    /// Channel and server-wide events return `None`. See [`crate::PeerKey`] for the distinction
    /// between durable Account pins and unverified Continuity pins. A `CHATHISTORY` replay carrying
    /// an `account` tag produces the same Account pin as a live line in the same authenticated
    /// scope; replay without that tag returns `None`.
    #[must_use]
    pub fn peer_key(&self) -> Option<&crate::PeerKey> {
        self.peer_key.as_deref()
    }

    /// Returns whether this live line proves the user is or is not a bot.
    #[must_use]
    pub const fn is_bot(&self) -> Option<bool> {
        self.is_bot
    }

    /// Returns the operator identity reported by this line.
    #[must_use]
    pub const fn oper(&self) -> Option<&Oper> {
        self.oper.as_ref()
    }
}

impl PartialEq for UserSource {
    fn eq(&self, other: &Self) -> bool {
        self.nick == other.nick
            && self.username == other.username
            && self.host == other.host
            && self.account == other.account
            && self.is_bot == other.is_bot
            && self.oper == other.oper
    }
}

impl Eq for UserSource {}

impl fmt::Display for UserSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.nick.fmt(formatter)
    }
}

impl papo_proto::MaskSource for UserSource {
    fn mask_parts(&self) -> Option<(&Nick, &str, &str)> {
        Some((self.nick(), self.username()?, self.host()?))
    }
}

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

pub(crate) fn user_source_from_line(
    prefix: LinePrefix<'_>,
    envelope: &super::EventEnvelope,
    casemapping: Casemapping,
    origin: crate::Origin,
    prove_not_bot: bool,
) -> Option<UserSource> {
    let source = match prefix {
        LinePrefix::Raw(prefix) => {
            let line = papo_proto::Line::new("TAGMSG")
                .param("*")
                .with_source(prefix);
            let source = line.as_ref().source()?;
            let papo_proto::Source::User {
                nick,
                username,
                host,
                ..
            } = source
            else {
                return None;
            };
            (
                nick.to_owned(),
                username.map(str::to_owned),
                host.map(str::to_owned),
            )
        }
        LinePrefix::Core(source) => match source {
            papo_core::Source::User {
                nick,
                username,
                host,
                ..
            } => (nick.into_string(), username, host),
            _ => return None,
        },
    };
    Some(UserSource {
        nick: origin.__nick(casemapping, source.0),
        username: source.1.filter(|_| source.2.is_some()),
        host: source.2,
        account: envelope.account.clone(),
        peer_key: None,
        is_bot: if envelope.bot {
            Some(true)
        } else {
            prove_not_bot.then_some(false)
        },
        oper: envelope.oper.clone(),
    })
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
    use std::sync::Arc;
    use std::time::SystemTime;

    use papo_proto::{Casemapping, Mask};

    use super::{Source, UserSource};
    use crate::{Account, Oper};

    #[test]
    fn exposes_each_line_fact_and_builds_complete_masks() {
        let mapping = Casemapping::Rfc1459;
        let account = mapping.account("alice");
        let source = UserSource {
            nick: mapping.nick("Alice[bot]"),
            username: Some("~alice".into()),
            host: Some("staff.example".into()),
            account: Account::LoggedIn(account.clone()),
            peer_key: None,
            is_bot: Some(true),
            oper: Some(Oper {
                name: Some("network-admin".into()),
            }),
        };

        assert_eq!(source.nick(), "alice{bot}");
        assert_eq!(source.username(), Some("~alice"));
        assert_eq!(source.host(), Some("staff.example"));
        assert_eq!(source.account().name(), Some(&account));
        assert_eq!(source.is_bot(), Some(true));
        assert_eq!(
            source.oper().and_then(|oper| oper.name.as_deref()),
            Some("network-admin")
        );
        assert_eq!(
            Mask::exact(&source).as_ref().map(Mask::as_str),
            Some("Alice[bot]!~alice@staff.example")
        );

        let mut pinned = source.clone();
        let scope = crate::peer_key::PeerScope::server("irc.example");
        let pin =
            crate::PeerKey::historical(Some(&scope), pinned.nick(), pinned.account()).unwrap();
        pinned.peer_key = Some(Arc::new(pin));
        assert_eq!(pinned, source);

        let mut different_nick = source.clone();
        different_nick.nick = mapping.nick("bob");
        let mut different_username = source.clone();
        different_username.username = Some("~operator".into());
        let mut different_host = source.clone();
        different_host.host = Some("users.example".into());
        let mut different_account = source.clone();
        different_account.account = Account::LoggedIn(mapping.account("bob-account"));
        let mut different_bot = source.clone();
        different_bot.is_bot = Some(false);
        let mut different_oper = source.clone();
        different_oper.oper = Some(Oper { name: None });
        for different in [
            different_nick,
            different_username,
            different_host,
            different_account,
            different_bot,
            different_oper,
        ] {
            assert_ne!(source, different);
        }
    }

    #[test]
    fn keeps_hosts_without_inventing_incomplete_usernames() {
        let mapping = Casemapping::Ascii;
        let source = UserSource {
            nick: mapping.nick("alice"),
            username: None,
            host: None,
            account: Account::Unknown,
            peer_key: None,
            is_bot: None,
            oper: None,
        };
        assert_eq!(source.username(), None);
        assert_eq!(source.host(), None);
        assert_eq!(source.account(), &Account::Unknown);
        assert_eq!(source.is_bot(), None);
        assert_eq!(source.oper(), None);
        assert_eq!(Mask::exact(&source), None);
    }

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

    #[test]
    fn maps_complete_core_user_sources() {
        let mapping = Casemapping::Rfc1459;
        let source = papo_core::Source::from_proto(
            papo_proto::Source::__user("Alice[bot]", Some("~alice"), Some("staff.example")),
            mapping,
        );
        let envelope = super::super::EventEnvelope {
            msgid: None,
            server_time: None,
            received_at: SystemTime::UNIX_EPOCH,
            account: Account::LoggedIn(mapping.account("alice-account")),
            bot: false,
            oper: None,
        };

        let mapped = super::user_source_from_line(
            super::LinePrefix::Core(source),
            &envelope,
            mapping,
            crate::Origin::NONE,
            true,
        )
        .unwrap();

        assert_eq!(mapped.nick(), "alice{bot}");
        assert_eq!(mapped.username(), Some("~alice"));
        assert_eq!(mapped.host(), Some("staff.example"));
        assert!(
            matches!(mapped.account(), Account::LoggedIn(account) if account == "alice-account")
        );
        assert_eq!(mapped.is_bot(), Some(false));
    }
}
