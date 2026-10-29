use std::{borrow::Cow, fmt};

macro_rules! protocol_token {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        ///
        /// Equality, ordering, and hashing use the exact wire spelling.
        #[derive(Clone, Debug, Eq, Hash, PartialEq)]
        pub struct $name(Cow<'static, str>);

        impl $name {
            #[allow(dead_code)]
            pub(crate) const fn constant(value: &'static str) -> Self {
                Self(Cow::Borrowed(value))
            }

            /// Returns the token's wire spelling.
            #[must_use]
            pub const fn as_str(&self) -> &str {
                match &self.0 {
                    Cow::Borrowed(value) => value,
                    Cow::Owned(value) => value.as_str(),
                }
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(Cow::Owned(value.to_owned()))
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(Cow::Owned(value))
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.as_str() == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.as_str() == *other
            }
        }
    };
}

protocol_token!(Command, "An IRC or CTCP command token.");

#[cfg(test)]
mod tests {
    use super::Command;

    #[test]
    fn tokens_preserve_spelling_and_compare_with_text() {
        let command = Command::from("version");

        assert_eq!(command.as_str(), "version");
        assert_eq!(command, "version");
        assert_ne!(command, "VERSION");
        assert_eq!(Command::constant("VERSION"), "VERSION");
        assert_eq!(command.to_string(), "version");
    }
}
