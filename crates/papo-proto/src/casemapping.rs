use std::{
    borrow::Cow,
    hash::{Hash, Hasher},
};

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
}

impl Casemapping {
    /// Parses a `CASEMAPPING` token value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ascii" => Some(Self::Ascii),
            "rfc1459" => Some(Self::Rfc1459),
            "rfc1459-strict" => Some(Self::Rfc1459Strict),
            _ => None,
        }
    }

    /// Returns the canonical spelling used to compare an IRC name.
    #[must_use]
    pub fn fold(self, name: &str) -> Cow<'_, str> {
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

/// An IRC name retaining its displayed spelling while comparing by its folded form.
#[derive(Clone, Debug)]
pub struct CaseKey {
    original: String,
    folded: String,
}

impl CaseKey {
    /// Creates a key using the server's casemapping.
    #[must_use]
    pub fn new(name: impl Into<String>, casemapping: Casemapping) -> Self {
        let original = name.into();
        let folded = casemapping.fold(&original).into_owned();
        Self { original, folded }
    }

    /// Returns the spelling received from the server or user.
    #[must_use]
    pub fn original(&self) -> &str {
        &self.original
    }

    /// Returns the canonical spelling used for equality and hashing.
    #[must_use]
    pub fn folded(&self) -> &str {
        &self.folded
    }
}

impl PartialEq for CaseKey {
    fn eq(&self, other: &Self) -> bool {
        self.folded == other.folded
    }
}

impl Eq for CaseKey {}

impl Hash for CaseKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.folded.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use std::{borrow::Cow, collections::HashMap};

    use super::{CaseKey, Casemapping};

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
        users.insert(CaseKey::new("Alice[bot]", Casemapping::Rfc1459), 1);
        let lookup = CaseKey::new("alice{bot}", Casemapping::Rfc1459);
        assert_eq!(users.get(&lookup), Some(&1));
        assert_eq!(users.keys().next().unwrap().original(), "Alice[bot]");
        assert_eq!(users.keys().next().unwrap().folded(), "alice{bot}");
    }
}
