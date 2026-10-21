use std::borrow::Cow;

#[cfg(feature = "precis")]
mod precis;

use crate::{AccountName, ChannelName, Mask, Nick};

/// A server's rules for comparing IRC names.
///
/// Servers define these rules. The default is [`Rfc1459`](Self::Rfc1459), as required before a
/// server advertises `CASEMAPPING` in `RPL_ISUPPORT`. Ergo advertises PRECIS through
/// `UTF8MAPPING`; the `Precis` variant and support for that advertisement require the `precis`
/// feature.
///
/// Applications using `papo` normally build names from text with the current or last-known rules
/// through its `State` name factories. The methods on this type are useful with saved rules,
/// offline data, and tests.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Casemapping {
    /// Only ASCII letters are case-insensitive.
    Ascii,
    /// ASCII letters and `[]\\~` are equivalent to `{}|^`.
    #[default]
    Rfc1459,
    /// ASCII letters and `[]\\` are equivalent to `{}|`.
    Rfc1459Strict,
    /// Unicode names are folded with the RFC 8265 `UsernameCaseMapped` profile.
    #[cfg(feature = "precis")]
    Precis,
}

impl Casemapping {
    /// Creates a nickname carrying these server-defined comparison rules.
    #[must_use]
    pub fn nick(self, text: impl AsRef<str>) -> Nick {
        Nick::new(text.as_ref().to_owned(), self)
    }

    /// Creates a channel name carrying these server-defined comparison rules.
    #[must_use]
    pub fn channel(self, text: impl AsRef<str>) -> ChannelName {
        ChannelName::new(text.as_ref().to_owned(), self)
    }

    /// Creates an account name carrying these server-defined comparison rules.
    #[must_use]
    pub fn account(self, text: impl AsRef<str>) -> AccountName {
        AccountName::new(text.as_ref().to_owned(), self)
    }

    /// Creates a mask carrying these server-defined comparison rules.
    #[must_use]
    pub fn mask(self, text: impl AsRef<str>) -> Mask {
        Mask::new(text.as_ref().to_owned(), self)
    }

    /// Parses a `CASEMAPPING` token value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ascii" => Some(Self::Ascii),
            "rfc1459" => Some(Self::Rfc1459),
            "rfc1459-strict" => Some(Self::Rfc1459Strict),
            #[cfg(feature = "precis")]
            "rfc7613" => Some(Self::Precis),
            _ => None,
        }
    }

    /// Returns the canonical spelling used to compare an IRC name.
    #[must_use]
    pub fn fold(self, name: &str) -> Cow<'_, str> {
        #[cfg(feature = "precis")]
        if matches!(self, Self::Precis) {
            return precis::fold(name);
        }

        let changed = name
            .chars()
            .any(|character| self.fold_char(character) != character);
        if !changed {
            return Cow::Borrowed(name);
        }

        Cow::Owned(
            name.chars()
                .map(|character| self.fold_char(character))
                .collect(),
        )
    }

    const fn fold_char(self, character: char) -> char {
        match character {
            'A'..='Z' => character.to_ascii_lowercase(),
            '[' if !matches!(self, Self::Ascii) => '{',
            ']' if !matches!(self, Self::Ascii) => '}',
            '\\' if !matches!(self, Self::Ascii) => '|',
            '~' if matches!(self, Self::Rfc1459) => '^',
            _ => character,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::Casemapping;

    #[test]
    fn folds_every_character_defined_by_each_mapping() {
        assert_eq!(
            Casemapping::Ascii.fold("ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
            "abcdefghijklmnopqrstuvwxyz"
        );
        assert_eq!(Casemapping::Ascii.fold("[]\\~{}|^"), "[]\\~{}|^");
        assert_eq!(Casemapping::Rfc1459.fold("AZ[]\\~"), "az{}|^");
        assert_eq!(Casemapping::Rfc1459.fold("09-_/"), "09-_/");
        assert_eq!(Casemapping::Rfc1459Strict.fold("AZ[]\\"), "az{}|");
        assert_eq!(Casemapping::Rfc1459Strict.fold("~^"), "~^");
    }

    #[test]
    fn parses_only_specified_token_values_and_defaults_to_rfc1459() {
        assert_eq!(Casemapping::parse("ascii"), Some(Casemapping::Ascii));
        assert_eq!(Casemapping::parse("rfc1459"), Some(Casemapping::Rfc1459));
        assert_eq!(
            Casemapping::parse("rfc1459-strict"),
            Some(Casemapping::Rfc1459Strict)
        );
        assert_eq!(Casemapping::parse("RFC1459"), None);
        assert_eq!(Casemapping::parse("unknown"), None);
        assert_eq!(Casemapping::default(), Casemapping::Rfc1459);
    }

    #[test]
    fn borrows_unchanged_names() {
        assert!(matches!(
            Casemapping::Rfc1459.fold("#ergo{dev}"),
            Cow::Borrowed(_)
        ));
    }
}
