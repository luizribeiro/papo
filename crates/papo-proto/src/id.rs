use std::{
    borrow::Borrow,
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    ops::Deref,
};

use crate::{Casemapping, Isupport};

/// Why an IRC identifier cannot be sent to the server.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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

macro_rules! original_text {
    ($name:ident, $field:tt) => {
        impl $name {
            /// Returns the owned identifier text.
            #[must_use]
            pub fn into_string(self) -> String {
                self.$field
            }
        }

        impl Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                &self.$field
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.$field
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.$field)
            }
        }

        impl From<&Self> for $name {
            fn from(value: &Self) -> Self {
                value.clone()
            }
        }
    };
}

macro_rules! identifier {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        ///
        /// Equality, ordering, and hashing use the exact received bytes.
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        original_text!($name, 0);

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
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
    };
}

macro_rules! name_identifier {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        ///
        /// The original spelling is retained for display. Equality, ordering, and hashing compare
        /// folded text using the server-defined casemapping captured at creation. Build a name
        /// from text with the matching [`Casemapping`](crate::Casemapping) method, or with a
        /// `papo::State` name factory when using the high-level client.
        #[derive(Clone, Debug)]
        pub struct $name {
            original: String,
            folded: String,
            casemapping: Casemapping,
        }

        impl $name {
            pub(crate) fn new(value: impl Into<String>, casemapping: Casemapping) -> Self {
                let original = value.into();
                let folded = casemapping.fold(&original).into_owned();
                Self {
                    original,
                    folded,
                    casemapping,
                }
            }

            /// Returns the server-defined rules captured when this name was created.
            #[must_use]
            pub const fn casemapping(&self) -> Casemapping {
                self.casemapping
            }
        }

        original_text!($name, original);

        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                self.folded == other.folded
            }
        }

        impl Eq for $name {}

        impl Hash for $name {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.folded.hash(state);
            }
        }

        impl PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        impl Ord for $name {
            fn cmp(&self, other: &Self) -> Ordering {
                self.folded.cmp(&other.folded)
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.folded == self.casemapping.fold(other)
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self == *other
            }
        }
    };
}

name_identifier!(Nick, "An IRC nickname.");
name_identifier!(ChannelName, "An IRC channel name.");
name_identifier!(AccountName, "An IRC account name.");
identifier!(Address, "An IRC wire address chosen by the caller.");
name_identifier!(Mask, "An IRC user or channel mask.");
identifier!(MetadataKey, "An IRC metadata key.");
identifier!(
    MessageId,
    "An opaque, network-unique IRC message identifier."
);

impl Nick {
    /// Validates this nickname against protocol characters and server limits.
    ///
    /// # Errors
    ///
    /// Returns an error when the nickname has an invalid character or prefix,
    /// or exceeds `NICKLEN` (nine characters when it is not advertised).
    #[doc(hidden)]
    pub fn check(&self, isupport: &Isupport) -> Result<(), InvalidIdentifier> {
        check_parameter(self)?;
        let first = self.chars().next().ok_or(InvalidIdentifier::Empty)?;
        if matches!(first, '$' | ':')
            || isupport.chantypes().contains(first)
            || isupport
                .prefix()
                .iter()
                .any(|prefix| prefix.symbol == first)
        {
            return Err(InvalidIdentifier::InvalidPrefix(first));
        }
        if let Some(character) = self.chars().find(|character| ",*?!@".contains(*character)) {
            return Err(InvalidIdentifier::InvalidCharacter(character));
        }
        check_length(self, isupport.nicklen().unwrap_or(9))
    }
}

impl ChannelName {
    /// Validates this channel name against `CHANTYPES` and `CHANNELLEN`.
    ///
    /// # Errors
    ///
    /// Returns an error when the name has no advertised channel prefix,
    /// contains a forbidden character, or exceeds the server's length limit.
    #[doc(hidden)]
    pub fn check(&self, isupport: &Isupport) -> Result<(), InvalidIdentifier> {
        let first = self.chars().next().ok_or(InvalidIdentifier::Empty)?;
        if !isupport.chantypes().contains(first) {
            return Err(InvalidIdentifier::InvalidPrefix(first));
        }
        if let Some(character) = self
            .chars()
            .find(|character| matches!(character, '\0' | '\r' | '\n' | ' ' | '\u{7}' | ','))
        {
            return Err(InvalidIdentifier::InvalidCharacter(character));
        }
        check_length(self, isupport.channellen().unwrap_or(50))
    }
}

impl Address {
    /// Validates this nickname, channel, status address, or server mask.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is invalid under the server's ISUPPORT state.
    #[doc(hidden)]
    pub fn check(&self, isupport: &Isupport) -> Result<(), InvalidIdentifier> {
        check_address(self, isupport)
    }
}

fn check_address(value: &str, isupport: &Isupport) -> Result<(), InvalidIdentifier> {
    let target = isupport
        .statusmsg()
        .and_then(|prefixes| value.strip_prefix(|character| prefixes.contains(character)))
        .unwrap_or(value);
    if isupport.is_channel(target) {
        return isupport.casemapping().channel(target).check(isupport);
    }
    if value == "*" || value.starts_with('$') {
        return check_parameter(value);
    }
    isupport.casemapping().nick(value).check(isupport)
}

fn check_length(value: &str, maximum: usize) -> Result<(), InvalidIdentifier> {
    let length = value.chars().count();
    if length > maximum {
        return Err(InvalidIdentifier::TooLong { length, maximum });
    }
    Ok(())
}

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

macro_rules! name_address_conversions {
    ($name:ident) => {
        impl From<Nick> for $name {
            fn from(value: Nick) -> Self {
                Self(value.into_string())
            }
        }

        impl From<&Nick> for $name {
            fn from(value: &Nick) -> Self {
                Self(value.as_ref().to_owned())
            }
        }

        impl From<ChannelName> for $name {
            fn from(value: ChannelName) -> Self {
                Self(value.into_string())
            }
        }

        impl From<&ChannelName> for $name {
            fn from(value: &ChannelName) -> Self {
                Self(value.as_ref().to_owned())
            }
        }
    };
}

name_address_conversions!(Address);

#[cfg(test)]
mod tests {
    use std::{
        borrow::Borrow,
        collections::{HashMap, hash_map::DefaultHasher},
        fmt,
        hash::{Hash, Hasher},
        ops::Deref,
    };

    use proptest::prelude::*;

    use crate::{Casemapping, Isupport, LineRef};

    use super::{Address, InvalidIdentifier, MessageId, MetadataKey, Nick};

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
    fn every_exact_identifier_has_the_owned_string_contract() {
        assert_contract::<Address>();
        assert_contract::<MetadataKey>();
        assert_contract::<MessageId>();
    }

    #[test]
    fn invalid_identifiers_explain_the_failed_boundary() {
        assert_eq!(
            InvalidIdentifier::TooLong {
                length: 31,
                maximum: 30,
            }
            .to_string(),
            "identifier has 31 characters; maximum is 30"
        );
    }

    #[test]
    fn names_preserve_spelling_and_compare_by_folded_text() {
        let nick = Casemapping::Rfc1459.nick("Alice[bot]");
        let mut values = HashMap::new();
        values.insert(nick.clone(), 7);

        assert_eq!(nick, "Alice[bot]");
        assert_eq!(nick, "alice{bot}");
        assert_eq!(nick.to_string(), "Alice[bot]");
        assert_eq!(values.get(&Casemapping::Ascii.nick("alice{bot}")), Some(&7));
        assert_eq!(nick.casemapping(), Casemapping::Rfc1459);
        assert_eq!(Nick::from(&nick).into_string(), "Alice[bot]");
    }

    #[test]
    fn names_use_every_literal_casemapping_rule() {
        assert_eq!(Casemapping::Rfc1459.nick("[]\\~"), "{}|^");
        assert_eq!(Casemapping::Rfc1459Strict.nick("[]\\"), "{}|");
        assert_ne!(Casemapping::Rfc1459Strict.nick("~"), "^");
        assert_eq!(Casemapping::Ascii.nick("BOB"), "bob");
        assert_ne!(Casemapping::Ascii.nick("[]\\~"), "{}|^");

        for mapping in [
            Casemapping::Ascii,
            Casemapping::Rfc1459,
            Casemapping::Rfc1459Strict,
        ] {
            assert_eq!(mapping.nick("bob"), "BOB");
        }
        #[cfg(feature = "precis")]
        assert_eq!(Casemapping::Precis.nick("bob"), "BOB");

        assert_eq!(Casemapping::Ascii.channel("#PAPO"), "#papo");
        assert_eq!(Casemapping::Ascii.account("EMILY"), "emily");
        assert_eq!(Casemapping::Ascii.mask("$*.EXAMPLE"), "$*.example");
    }

    proptest! {
        #[test]
        fn equal_names_always_hash_equally(
            text in "[A-Za-z0-9\\[\\]\\\\~{}|^_-]{0,32}",
            first in 0_u8..3,
            second in 0_u8..3,
        ) {
            let mappings = [
                Casemapping::Ascii,
                Casemapping::Rfc1459,
                Casemapping::Rfc1459Strict,
            ];
            let left = mappings[usize::from(first)].nick(&text);
            let right = mappings[usize::from(second)].nick(&text);
            if left == right {
                let mut left_hash = DefaultHasher::new();
                left.hash(&mut left_hash);
                let mut right_hash = DefaultHasher::new();
                right.hash(&mut right_hash);
                prop_assert_eq!(left_hash.finish(), right_hash.finish());
            }
        }
    }

    #[test]
    fn addresses_accept_typed_nicks_and_channels() {
        assert_eq!(Address::from(Casemapping::default().nick("alice")), "alice");
        assert_eq!(
            Address::from(&Casemapping::default().channel("#papo")),
            "#papo"
        );
    }

    #[test]
    fn validates_exact_and_structured_identifiers() {
        assert_eq!(Casemapping::default().account("emily").check(), Ok(()));
        assert_eq!(
            Casemapping::default().mask("*!*@cloak.example").check(),
            Ok(())
        );
        assert_eq!(MetadataKey::from("profile/display-name").check(), Ok(()));
        assert_eq!(MessageId::from("8VX4P6dUQ44L").check(), Ok(()));

        assert_eq!(
            Casemapping::default().account("*").check(),
            Err(InvalidIdentifier::InvalidCharacter('*'))
        );
        assert_eq!(
            Casemapping::default().mask("").check(),
            Err(InvalidIdentifier::Empty)
        );
        assert_eq!(
            MetadataKey::from("Avatar").check(),
            Err(InvalidIdentifier::InvalidCharacter('A'))
        );
        assert_eq!(
            MessageId::from(":8VX4P6dUQ44L").check(),
            Err(InvalidIdentifier::InvalidPrefix(':'))
        );
        assert_eq!(
            MessageId::from("8VX4 P6dUQ44L").check(),
            Err(InvalidIdentifier::InvalidCharacter(' '))
        );
    }

    #[test]
    fn rejects_every_parameter_delimiter_and_empty_opaque_value() {
        for (value, character) in [
            ("emily\0staff", '\0'),
            ("emily\rstaff", '\r'),
            ("emily\nstaff", '\n'),
            ("emily staff", ' '),
        ] {
            assert_eq!(
                Casemapping::default().account(value).check(),
                Err(InvalidIdentifier::InvalidCharacter(character))
            );
        }
        assert_eq!(
            Casemapping::default().account("").check(),
            Err(InvalidIdentifier::Empty)
        );
        assert_eq!(
            Casemapping::default().mask(":*!*@localhost").check(),
            Err(InvalidIdentifier::InvalidPrefix(':'))
        );
        assert_eq!(
            Casemapping::default().mask("*!*@local host").check(),
            Err(InvalidIdentifier::InvalidCharacter(' '))
        );
        assert_eq!(MessageId::from("").check(), Err(InvalidIdentifier::Empty));
        assert_eq!(
            MessageId::from("RfcBTzyx\nY120004").check(),
            Err(InvalidIdentifier::InvalidCharacter('\n'))
        );

        assert_eq!(MetadataKey::from("a0_./-").check(), Ok(()));
        assert_eq!(MetadataKey::from("").check(), Err(InvalidIdentifier::Empty));
        assert_eq!(
            MetadataKey::from("profile+color").check(),
            Err(InvalidIdentifier::InvalidCharacter('+'))
        );
    }

    fn libera_isupport() -> Isupport {
        let line = LineRef::parse(
            ":irc.libera.chat 005 papo CASEMAPPING=rfc1459 CHANTYPES=# PREFIX=(ov)@+ NICKLEN=16 CHANNELLEN=50 STATUSMSG=@+ :are supported",
        )
        .unwrap();
        let mut support = Isupport::default();
        support.apply(&line);
        support
    }

    #[test]
    fn validates_names_at_literal_isupport_boundaries() {
        let support = libera_isupport();
        assert_eq!(
            support
                .casemapping()
                .nick("abcdefghijklmnop")
                .check(&support),
            Ok(())
        );
        assert_eq!(
            support
                .casemapping()
                .nick("éééééééééééééééé")
                .check(&support),
            Ok(())
        );
        assert_eq!(
            support
                .casemapping()
                .nick("abcdefghijklmnopq")
                .check(&support),
            Err(InvalidIdentifier::TooLong {
                length: 17,
                maximum: 16
            })
        );
        assert_eq!(
            support
                .casemapping()
                .channel(format!("#{}", "p".repeat(49)))
                .check(&support),
            Ok(())
        );
        assert_eq!(
            support
                .casemapping()
                .channel(format!("#{}", "p".repeat(50)))
                .check(&support),
            Err(InvalidIdentifier::TooLong {
                length: 51,
                maximum: 50
            })
        );
    }

    #[test]
    fn validates_literal_default_name_limits() {
        let support = Isupport::default();
        assert_eq!(
            support.casemapping().nick("abcdefghi").check(&support),
            Ok(())
        );
        assert_eq!(
            support.casemapping().nick("abcdefghij").check(&support),
            Err(InvalidIdentifier::TooLong {
                length: 10,
                maximum: 9,
            })
        );
        assert_eq!(
            support
                .casemapping()
                .channel(format!("#{}", "p".repeat(49)))
                .check(&support),
            Ok(())
        );
        assert_eq!(
            support
                .casemapping()
                .channel(format!("#{}", "p".repeat(50)))
                .check(&support),
            Err(InvalidIdentifier::TooLong {
                length: 51,
                maximum: 50,
            })
        );
    }

    #[test]
    fn rejects_name_delimiters_and_unadvertised_prefixes() {
        let support = libera_isupport();
        for (nick, character) in [
            ("alice\0staff", '\0'),
            ("alice\rstaff", '\r'),
            ("alice\nstaff", '\n'),
            ("alice staff", ' '),
        ] {
            assert_eq!(
                support.casemapping().nick(nick).check(&support),
                Err(InvalidIdentifier::InvalidCharacter(character))
            );
        }
        for (nick, character) in [
            ("alice,bob", ','),
            ("alice*", '*'),
            ("alice?", '?'),
            ("alice!u", '!'),
            ("alice@h", '@'),
        ] {
            assert_eq!(
                support.casemapping().nick(nick).check(&support),
                Err(InvalidIdentifier::InvalidCharacter(character))
            );
        }
        for (nick, prefix) in [
            ("#alice", '#'),
            ("+alice", '+'),
            ("$alice", '$'),
            (":alice", ':'),
        ] {
            assert_eq!(
                support.casemapping().nick(nick).check(&support),
                Err(InvalidIdentifier::InvalidPrefix(prefix))
            );
        }
        assert_eq!(
            support.casemapping().channel("papo").check(&support),
            Err(InvalidIdentifier::InvalidPrefix('p'))
        );
        for (channel, character) in [
            ("#papo\0dev", '\0'),
            ("#papo\rdev", '\r'),
            ("#papo\ndev", '\n'),
            ("#papo dev", ' '),
            ("#papo\u{7}dev", '\u{7}'),
            ("#papo,dev", ','),
        ] {
            assert_eq!(
                support.casemapping().channel(channel).check(&support),
                Err(InvalidIdentifier::InvalidCharacter(character))
            );
        }
    }

    #[test]
    fn validates_each_supported_address_shape() {
        let support = libera_isupport();
        for target in ["Alice", "#papo", "@#papo", "$*.example", "*"] {
            assert_eq!(Address::from(target).check(&support), Ok(()));
        }
        assert_eq!(
            Address::from("@Alice").check(&support),
            Err(InvalidIdentifier::InvalidPrefix('@'))
        );
        assert_eq!(
            Address::from("#papo,dev").check(&support),
            Err(InvalidIdentifier::InvalidCharacter(','))
        );
    }
}
