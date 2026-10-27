use std::{borrow::Cow, fmt, str::FromStr};

#[cfg(feature = "precis")]
mod precis;

use crate::{AccountName, ChannelName, Mask, Nick, Origin};

/// A server's rules for comparing IRC names.
///
/// Servers define these rules. The default is [`Rfc1459`](Self::Rfc1459), as required before a
/// server advertises `CASEMAPPING` in `RPL_ISUPPORT`. Ergo advertises PRECIS through
/// `UTF8MAPPING`; the `Precis` variant and support for that advertisement require the `precis`
/// feature. Without it, `UTF8MAPPING=rfc8265` falls back to the advertised `CASEMAPPING` (or
/// RFC 1459 when absent), and [`papo::Warning::Utf8CasemappingUnsupported`](https://docs.rs/papo/latest/papo/enum.Warning.html#variant.Utf8CasemappingUnsupported)
/// is emitted once per connection.
///
/// Applications using `papo` build typed names through [`papo::State`](https://docs.rs/papo/latest/papo/struct.State.html)
/// so the result also records which client produced it. Seed saved comparison rules with
/// [`papo::Builder::casemapping`](https://docs.rs/papo/latest/papo/struct.Builder.html#method.casemapping).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
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

/// An error returned for an unknown IRC casemapping name.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct UnknownCasemapping;

impl fmt::Display for UnknownCasemapping {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unknown IRC casemapping")
    }
}

impl std::error::Error for UnknownCasemapping {}

impl Casemapping {
    /// Returns the canonical token spelling for these comparison rules.
    ///
    /// `rfc7613` is papo's persistent spelling for `Precis`, even though servers such as
    /// Ergo advertise those rules through `UTF8MAPPING`. A build without the `precis` feature
    /// cannot parse that spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ascii => "ascii",
            Self::Rfc1459 => "rfc1459",
            Self::Rfc1459Strict => "rfc1459-strict",
            #[cfg(feature = "precis")]
            Self::Precis => "rfc7613",
        }
    }

    /// Creates a nickname carrying these server-defined comparison rules.
    #[doc(hidden)]
    #[must_use]
    pub fn nick(self, text: impl Into<String>) -> Nick {
        Origin::NONE.__nick(self, text)
    }

    /// Creates a channel name carrying these server-defined comparison rules.
    #[doc(hidden)]
    #[must_use]
    pub fn channel(self, text: impl Into<String>) -> ChannelName {
        Origin::NONE.__channel(self, text)
    }

    /// Creates an account name carrying these server-defined comparison rules.
    #[doc(hidden)]
    #[must_use]
    pub fn account(self, text: impl Into<String>) -> AccountName {
        Origin::NONE.__account(self, text)
    }

    /// Creates a mask carrying these server-defined comparison rules.
    #[doc(hidden)]
    #[must_use]
    pub fn mask(self, text: impl Into<String>) -> Mask {
        Origin::NONE.__mask(self, text)
    }

    /// Returns the canonical spelling used to compare an IRC name.
    ///
    /// Use this when a persistent key must follow saved server comparison rules without carrying
    /// a running client's identity.
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

impl FromStr for Casemapping {
    type Err = UnknownCasemapping;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ascii" => Ok(Self::Ascii),
            "rfc1459" => Ok(Self::Rfc1459),
            "rfc1459-strict" => Ok(Self::Rfc1459Strict),
            #[cfg(feature = "precis")]
            "rfc7613" => Ok(Self::Precis),
            _ => Err(UnknownCasemapping),
        }
    }
}

impl fmt::Display for Casemapping {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::{Casemapping, UnknownCasemapping};

    #[test]
    fn canonical_spellings_round_trip_every_variant() {
        let mappings = [
            Casemapping::Ascii,
            Casemapping::Rfc1459,
            Casemapping::Rfc1459Strict,
        ];

        for mapping in mappings {
            assert_eq!(mapping.as_str().parse(), Ok(mapping));
            assert_eq!(mapping.to_string(), mapping.as_str());
        }
    }

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
        assert_eq!("ascii".parse(), Ok(Casemapping::Ascii));
        assert_eq!("rfc1459".parse(), Ok(Casemapping::Rfc1459));
        assert_eq!("rfc1459-strict".parse(), Ok(Casemapping::Rfc1459Strict));
        assert_eq!("RFC1459".parse::<Casemapping>(), Err(UnknownCasemapping));
        assert_eq!("unknown".parse::<Casemapping>(), Err(UnknownCasemapping));
        assert_eq!(UnknownCasemapping.to_string(), "unknown IRC casemapping");
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
