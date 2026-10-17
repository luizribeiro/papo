use std::{borrow::Borrow, fmt, ops::Deref};

/// Why an IRC identifier cannot be sent to the server.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InvalidIdentifier {
    /// The identifier is empty.
    Empty,
    /// The identifier contains a character forbidden in its protocol context.
    InvalidCharacter(char),
    /// The identifier starts with a character forbidden in its protocol context.
    InvalidPrefix(char),
    /// The identifier exceeds a server or protocol length limit.
    TooLong {
        /// The identifier's character count.
        length: usize,
        /// The maximum accepted character count.
        maximum: usize,
    },
}

impl fmt::Display for InvalidIdentifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("identifier is empty"),
            Self::InvalidCharacter(character) => {
                write!(
                    formatter,
                    "identifier contains forbidden character {character:?}"
                )
            }
            Self::InvalidPrefix(character) => {
                write!(
                    formatter,
                    "identifier starts with forbidden character {character:?}"
                )
            }
            Self::TooLong { length, maximum } => {
                write!(
                    formatter,
                    "identifier has {length} characters; maximum is {maximum}"
                )
            }
        }
    }
}

impl std::error::Error for InvalidIdentifier {}

macro_rules! identifier {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        ///
        /// Equality, ordering, and hashing use the exact received bytes. Use
        /// [`crate::Casemapping`] or [`crate::Folded`] for IRC name comparison.
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Returns the owned identifier text.
            #[must_use]
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&String> for $name {
            fn from(value: &String) -> Self {
                Self(value.clone())
            }
        }

        impl From<&Self> for $name {
            fn from(value: &Self) -> Self {
                value.clone()
            }
        }
    };
}

identifier!(Nick, "An IRC nickname.");
identifier!(ChannelName, "An IRC channel name.");
identifier!(AccountName, "An IRC account name.");
identifier!(Target, "An IRC message target chosen by the caller.");
identifier!(Mask, "An IRC user or channel mask.");
identifier!(MetadataKey, "An IRC metadata key.");
identifier!(
    MessageId,
    "An opaque, network-unique IRC message identifier."
);

impl AccountName {
    /// Validates this account name for use as an IRC parameter.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty name, the logout marker, or wire delimiters.
    pub fn check(&self) -> Result<(), InvalidIdentifier> {
        check_parameter(self)?;
        if self.as_ref() == "*" {
            return Err(InvalidIdentifier::InvalidCharacter('*'));
        }
        Ok(())
    }
}

impl Mask {
    /// Validates this mask for use as an IRC parameter.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty mask or wire delimiters.
    pub fn check(&self) -> Result<(), InvalidIdentifier> {
        check_parameter(self)
    }
}

impl MetadataKey {
    /// Validates the metadata key character set.
    ///
    /// # Errors
    ///
    /// Returns an error unless the key is nonempty and contains only lowercase
    /// ASCII letters, digits, `_`, `.`, `/`, or `-`.
    pub fn check(&self) -> Result<(), InvalidIdentifier> {
        if self.is_empty() {
            return Err(InvalidIdentifier::Empty);
        }
        self.chars()
            .find(|character| {
                !character.is_ascii_lowercase()
                    && !character.is_ascii_digit()
                    && !matches!(character, '_' | '.' | '/' | '-')
            })
            .map_or(Ok(()), |character| {
                Err(InvalidIdentifier::InvalidCharacter(character))
            })
    }
}

impl MessageId {
    /// Validates this message identifier for use as an IRC parameter.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty identifier or wire delimiters.
    pub fn check(&self) -> Result<(), InvalidIdentifier> {
        check_parameter(self)
    }
}

fn check_parameter(value: &str) -> Result<(), InvalidIdentifier> {
    if value.is_empty() {
        return Err(InvalidIdentifier::Empty);
    }
    if value.starts_with(':') {
        return Err(InvalidIdentifier::InvalidPrefix(':'));
    }
    value
        .chars()
        .find(|character| matches!(character, '\0' | '\r' | '\n' | ' '))
        .map_or(Ok(()), |character| {
            Err(InvalidIdentifier::InvalidCharacter(character))
        })
}

impl From<Nick> for Target {
    fn from(value: Nick) -> Self {
        Self(value.into_string())
    }
}

impl From<&Nick> for Target {
    fn from(value: &Nick) -> Self {
        Self(value.as_ref().to_owned())
    }
}

impl From<ChannelName> for Target {
    fn from(value: ChannelName) -> Self {
        Self(value.into_string())
    }
}

impl From<&ChannelName> for Target {
    fn from(value: &ChannelName) -> Self {
        Self(value.as_ref().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::{borrow::Borrow, collections::HashMap, fmt, hash::Hash, ops::Deref};

    use super::{AccountName, ChannelName, Mask, MessageId, MetadataKey, Nick, Target};

    fn assert_contract<T>()
    where
        T: Clone
            + fmt::Debug
            + fmt::Display
            + Eq
            + Hash
            + Ord
            + Deref<Target = str>
            + AsRef<str>
            + Borrow<str>
            + From<String>
            + for<'a> From<&'a str>
            + for<'a> From<&'a String>
            + for<'a> From<&'a T>
            + PartialEq<str>
            + for<'a> PartialEq<&'a str>,
    {
    }

    #[test]
    fn every_identifier_has_the_owned_string_contract() {
        assert_contract::<Nick>();
        assert_contract::<ChannelName>();
        assert_contract::<AccountName>();
        assert_contract::<Target>();
        assert_contract::<Mask>();
        assert_contract::<MetadataKey>();
        assert_contract::<MessageId>();
    }

    #[test]
    fn identifiers_preserve_case_and_compare_exact_bytes() {
        let nick = Nick::from("Alice[bot]");
        let mut values = HashMap::new();
        values.insert(nick.clone(), 7);

        assert_eq!(nick, "Alice[bot]");
        assert_ne!(nick, "alice{bot}");
        assert_eq!(nick.to_string(), "Alice[bot]");
        assert_eq!(format!("{nick:?}"), "Nick(\"Alice[bot]\")");
        assert_eq!(values.get("Alice[bot]"), Some(&7));
        assert_eq!(Nick::from(&nick).into_string(), "Alice[bot]");
    }

    #[test]
    fn targets_accept_typed_nicks_and_channels() {
        assert_eq!(Target::from(Nick::from("alice")), "alice");
        assert_eq!(Target::from(&ChannelName::from("#papo")), "#papo");
    }
}
