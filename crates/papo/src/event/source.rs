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

#[doc(inline)]
pub use papo_core::Source;

pub(crate) fn from_proto_with_origin(
    source: papo_proto::Source<'_>,
    casemapping: Casemapping,
    origin: crate::Origin,
) -> Source {
    papo_core::Source::__from_proto(source, casemapping, origin)
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

    use super::UserSource;
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
}
