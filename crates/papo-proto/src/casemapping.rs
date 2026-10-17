use std::{
    borrow::{Borrow, Cow},
    cmp::Ordering,
    hash::{Hash, Hasher},
};

#[cfg(feature = "precis")]
use precis_profiles::{UsernameCaseMapped, precis_core::profile::PrecisFastInvocation as _};

/// A server's rules for comparing IRC nicknames and channel names.
///
/// The default is [`Rfc1459`](Self::Rfc1459), as required when a server has not
/// advertised `CASEMAPPING` in `RPL_ISUPPORT`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
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
            return UsernameCaseMapped::enforce(name).unwrap_or_else(|_| {
                // Ergo rejects these names, but map keys still need one deterministic fold.
                Self::Ascii.fold(name)
            });
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

    /// Reports whether two IRC names are equal under this mapping.
    #[must_use]
    pub fn eq(self, left: &str, right: &str) -> bool {
        self.fold(left) == self.fold(right)
    }

    /// Creates a casemapped key while retaining the typed original value.
    #[must_use]
    pub fn key<T>(self, value: &T) -> Folded<T>
    where
        T: AsRef<str> + Clone,
    {
        Folded::new(value.clone(), self)
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

/// A typed IRC value retaining its spelling while comparing by its folded form.
#[derive(Clone, Debug)]
pub struct Folded<T> {
    original: T,
    folded: String,
}

impl<T: AsRef<str>> Folded<T> {
    /// Creates a key using the server's casemapping.
    #[must_use]
    pub fn new(value: impl Into<T>, casemapping: Casemapping) -> Self {
        let original = value.into();
        let folded = casemapping.fold(original.as_ref()).into_owned();
        Self { original, folded }
    }

    /// Returns the spelling received from the server or user.
    #[must_use]
    pub fn original(&self) -> &str {
        self.original.as_ref()
    }

    /// Returns the typed value received from the server or user.
    #[must_use]
    pub const fn value(&self) -> &T {
        &self.original
    }

    /// Returns the canonical spelling used for equality and hashing.
    #[must_use]
    pub fn folded(&self) -> &str {
        &self.folded
    }
}

impl<T> PartialEq for Folded<T> {
    fn eq(&self, other: &Self) -> bool {
        self.folded == other.folded
    }
}

impl<T> Eq for Folded<T> {}

impl<T> Hash for Folded<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.folded.hash(state);
    }
}

impl<T> PartialOrd for Folded<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Folded<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.folded.cmp(&other.folded)
    }
}

impl<T> Borrow<str> for Folded<T> {
    fn borrow(&self) -> &str {
        &self.folded
    }
}

#[cfg(test)]
mod tests {
    use std::{
        borrow::Cow,
        collections::{HashMap, hash_map::DefaultHasher},
        hash::{Hash, Hasher},
    };

    use crate::Nick;

    use super::{Casemapping, Folded};

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
    fn compares_ergo_names_and_borrows_unchanged_names() {
        assert!(Casemapping::Ascii.eq("dan-Howl", "dan-howl"));
        assert!(Casemapping::Rfc1459.eq("#Ergo[Dev]", "#ergo{dev}"));
        assert!(!Casemapping::Ascii.eq("#Ergo[Dev]", "#ergo{dev}"));
        assert!(matches!(
            Casemapping::Rfc1459.fold("#ergo{dev}"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn case_keys_hash_by_folded_name_and_retain_spelling() {
        let mut users = HashMap::new();
        users.insert(Folded::<String>::new("Alice[bot]", Casemapping::Rfc1459), 1);
        let lookup = Folded::<String>::new("alice{bot}", Casemapping::Rfc1459);
        assert_eq!(users.get(&lookup), Some(&1));
        assert_eq!(users.keys().next().unwrap().original(), "Alice[bot]");
        assert_eq!(users.keys().next().unwrap().folded(), "alice{bot}");

        let other = Folded::<String>::new("Bob", Casemapping::Rfc1459);
        assert_ne!(lookup, other);
        let mut lookup_hash = DefaultHasher::new();
        lookup.hash(&mut lookup_hash);
        let mut other_hash = DefaultHasher::new();
        other.hash(&mut other_hash);
        assert_ne!(lookup_hash.finish(), other_hash.finish());
    }

    #[test]
    fn folded_keys_follow_each_irc_casemapping() {
        let rfc = Casemapping::Rfc1459.key(&Nick::from("[]\\~"));
        let rfc_equivalent = Casemapping::Rfc1459.key(&Nick::from("{}|^"));
        assert_eq!(rfc, rfc_equivalent);

        let strict = Casemapping::Rfc1459Strict.key(&Nick::from("[]\\~"));
        let strict_different = Casemapping::Rfc1459Strict.key(&Nick::from("{}|^"));
        assert_ne!(strict, strict_different);
        assert_eq!(strict.folded(), "{}|~");

        let ascii = Casemapping::Ascii.key(&Nick::from("ALICE[]\\~"));
        let ascii_case_only = Casemapping::Ascii.key(&Nick::from("alice[]\\~"));
        let ascii_punctuation = Casemapping::Ascii.key(&Nick::from("alice{}|^"));
        assert_eq!(ascii, ascii_case_only);
        assert_ne!(ascii, ascii_punctuation);
        assert_eq!(ascii.value().as_ref(), "ALICE[]\\~");
    }

    #[test]
    fn folded_keys_borrow_the_canonical_spelling() {
        let mut users = HashMap::new();
        users.insert(Casemapping::Rfc1459.key(&Nick::from("Alice[bot]")), 7);

        assert_eq!(users.get("alice{bot}"), Some(&7));
        assert_eq!(users.get("Alice[bot]"), None);
    }

    #[cfg(feature = "precis")]
    #[test]
    fn folds_unicode_nicks_with_ergo_precis_rules() {
        assert_eq!(Casemapping::parse("rfc7613"), Some(Casemapping::Precis));
        assert_eq!(Casemapping::Precis.fold("Ünïcode"), "ünïcode");
        assert_eq!(Casemapping::Precis.fold("İstanbul"), "i\u{307}stanbul");
        assert!(Casemapping::Precis.eq("ÜNÏCODE", "ünïcode"));
        assert!(!Casemapping::Precis.eq("İstanbul", "istanbul"));
        assert!(matches!(
            Casemapping::Precis.fold("ünïcode"),
            Cow::Borrowed(_)
        ));
    }

    #[cfg(feature = "precis")]
    #[test]
    fn folds_precis_rejections_as_ascii() {
        assert_eq!(Casemapping::Precis.fold("Bad Nick"), "bad nick");
        assert_eq!(Casemapping::Precis.fold("Bad\u{7}Nick"), "bad\u{7}nick");
    }
}
