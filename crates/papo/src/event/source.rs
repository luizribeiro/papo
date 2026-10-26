use std::fmt;

use crate::{Account, Casemapping, Nick, Oper};

/// A user prefix and the identity facts carried by the same IRC line.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct UserSource {
    nick: Nick,
    username: Option<String>,
    host: Option<String>,
    account: Account,
    is_bot: Option<bool>,
    oper: Option<Oper>,
}

impl UserSource {
    pub(crate) fn set_account(&mut self, account: Account) {
        self.account = account;
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
            is_bot: None,
            oper: None,
        }),
        papo_proto::Source::Server(server) => Source::Server(server.into()),
    }
}

pub(crate) fn from_proto_with_account(
    source: papo_proto::Source<'_>,
    casemapping: Casemapping,
    origin: crate::Origin,
    account: Account,
) -> Source {
    let mut source = from_proto_with_origin(source, casemapping, origin);
    if let Source::User(user) = &mut source {
        user.set_account(account);
    }
    source
}

pub(crate) fn user_from_line(
    line: &papo_proto::LineRef<'_>,
    casemapping: Casemapping,
    origin: crate::Origin,
    account_tag_enabled: bool,
    prove_not_bot: bool,
) -> Option<UserSource> {
    let papo_proto::Source::User {
        nick,
        username,
        host,
        ..
    } = line.source()?
    else {
        return None;
    };
    let account = line.tag("account").map_or_else(
        || {
            if account_tag_enabled {
                Account::LoggedOut
            } else {
                Account::Unknown
            }
        },
        |account| Account::LoggedIn(origin.__account(casemapping, account)),
    );
    let is_bot = if line.tags().any(|(key, _)| key == "bot") {
        Some(true)
    } else {
        prove_not_bot.then_some(false)
    };
    let oper = line
        .tags()
        .find(|(key, _)| *key == "draft/oper")
        .map(|(_, value)| Oper {
            name: value.map(std::borrow::Cow::into_owned),
        });
    Some(UserSource {
        nick: origin.__nick(casemapping, nick),
        username: username.filter(|_| host.is_some()).map(str::to_owned),
        host: host.map(str::to_owned),
        account,
        is_bot,
        oper,
    })
}

pub(crate) fn user_from_prefix(
    prefix: &str,
    casemapping: Casemapping,
    origin: crate::Origin,
    account: Account,
    is_bot: Option<bool>,
    oper: Option<Oper>,
) -> Option<UserSource> {
    let line = papo_proto::Line::new("TAGMSG")
        .param("*")
        .with_source(prefix);
    let papo_proto::Source::User {
        nick,
        username,
        host,
        ..
    } = line.as_ref().source()?
    else {
        return None;
    };
    Some(UserSource {
        nick: origin.__nick(casemapping, nick),
        username: username.filter(|_| host.is_some()).map(str::to_owned),
        host: host.map(str::to_owned),
        account,
        is_bot,
        oper,
    })
}

pub(crate) fn from_core(source: papo_core::Source, origin: crate::Origin) -> Option<Source> {
    match source {
        papo_core::Source::User {
            nick,
            username,
            host,
            ..
        } => Some(Source::User(UserSource {
            nick: origin.__nick(nick.casemapping(), nick.as_ref()),
            username: username.filter(|_| host.is_some()),
            host,
            account: Account::Unknown,
            is_bot: None,
            oper: None,
        })),
        papo_core::Source::Server { name, .. } => Some(Source::Server(name)),
        _ => None,
    }
}

pub(crate) fn from_core_with_account(
    source: papo_core::Source,
    origin: crate::Origin,
    account: Account,
) -> Option<Source> {
    let mut source = from_core(source, origin)?;
    if let Source::User(user) = &mut source {
        user.set_account(account);
    }
    Some(source)
}

pub(crate) fn from_prefix(
    prefix: String,
    casemapping: Casemapping,
    origin: crate::Origin,
) -> Source {
    let message = papo_proto::Line::new("NOTICE")
        .with_params(["*"])
        .with_source(prefix);
    from_proto_with_origin(
        message.as_ref().source().expect("owned source is present"),
        casemapping,
        origin,
    )
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
            Mask::exact(&source).as_deref(),
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
