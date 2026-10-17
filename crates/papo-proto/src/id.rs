use std::{borrow::Borrow, fmt, ops::Deref};

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
