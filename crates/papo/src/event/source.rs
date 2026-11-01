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
    pub(crate) fn set_account(&mut self, account: Account) {
        self.account = account;
    }

    pub(crate) fn set_peer_key(&mut self, peer_key: crate::peer_key::PeerKey) {
        self.peer_key = Some(Arc::new(peer_key));
    }

    pub(crate) fn from_state(
        nick: &Nick,
        username: Option<String>,
        host: Option<String>,
        account: Account,
        origin: crate::Origin,
    ) -> Self {
        Self {
            nick: origin.__nick(nick.casemapping(), nick.as_str()),
            username: username.filter(|_| host.is_some()),
            host,
            account,
            peer_key: None,
            is_bot: None,
            oper: None,
        }
    }

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
    /// between durable Account pins and unverified Continuity pins.
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
    /// A user and the identity facts carried by the line.
    User(UserSource),
    /// A server name.
    Server(crate::ServerName),
}

impl Source {
    /// Returns the nickname for a user source.
    #[must_use]
    pub const fn nick(&self) -> Option<&Nick> {
        match self {
            Self::User(user) => Some(user.nick()),
            Self::Server(_) => None,
        }
    }

    /// Returns the user source when the actor is a user.
    #[must_use]
    pub const fn user(&self) -> Option<&UserSource> {
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

impl papo_proto::MaskSource for Source {
    fn mask_parts(&self) -> Option<(&Nick, &str, &str)> {
        self.user()?.mask_parts()
    }
}

pub(crate) fn from_proto_with_origin(
    source: papo_proto::Source<'_>,
    casemapping: Casemapping,
    origin: crate::Origin,
) -> Source {
    match source {
        papo_proto::Source::User {
            nick,
            username,
            host,
            ..
        } => Source::User(UserSource {
            nick: origin.__nick(casemapping, nick),
            username: username.filter(|_| host.is_some()).map(str::to_owned),
            host: host.map(str::to_owned),
            account: Account::Unknown,
            peer_key: None,
            is_bot: None,
            oper: None,
        }),
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
    prove_not_bot: bool,
) -> Option<Source> {
    let source = match prefix {
        LinePrefix::Raw(prefix) => {
            let line = papo_proto::Line::new("TAGMSG")
                .param("*")
                .with_source(prefix);
            from_proto_with_origin(line.as_ref().source()?, casemapping, origin)
        }
        LinePrefix::Core(source) => from_core(source, origin)?,
    };
    let Source::User(mut user) = source else {
        return Some(source);
    };
    user.account = envelope.account.clone();
    user.is_bot = if envelope.bot {
        Some(true)
    } else {
        prove_not_bot.then_some(false)
    };
    user.oper.clone_from(&envelope.oper);
    Some(Source::User(user))
}

pub(crate) fn from_core(source: papo_core::Source, origin: crate::Origin) -> Option<Source> {
    match source {
        papo_core::Source::User {
            nick,
            username,
            host,
            ..
        } => Some(Source::User(UserSource {
            nick: origin.__nick(nick.casemapping(), nick.as_str()),
            username: username.filter(|_| host.is_some()),
            host,
            account: Account::Unknown,
            peer_key: None,
            is_bot: None,
            oper: None,
        })),
        papo_core::Source::Server { name, .. } => Some(Source::Server(name)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
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
}
