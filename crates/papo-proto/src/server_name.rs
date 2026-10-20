use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    ops::Deref,
};

/// An IRC server name.
///
/// The original spelling is retained for display. Equality, ordering, and hashing compare ASCII
/// case-insensitively, as required for server names by the IRC protocol.
#[derive(Clone, Debug)]
pub struct ServerName {
    original: String,
    folded: String,
}

impl ServerName {
    /// Returns the owned server name text.
    #[must_use]
    pub fn into_string(self) -> String {
        self.original
    }
}
impl PartialEq for ServerName {
    fn eq(&self, other: &Self) -> bool {
        self.folded == other.folded
    }
}
impl Eq for ServerName {}
impl Hash for ServerName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.folded.hash(state);
    }
}
impl PartialOrd for ServerName {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ServerName {
    fn cmp(&self, other: &Self) -> Ordering {
        self.folded.cmp(&other.folded)
    }
}
impl PartialEq<str> for ServerName {
    fn eq(&self, other: &str) -> bool {
        self.original.eq_ignore_ascii_case(other)
    }
}
impl PartialEq<&str> for ServerName {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}
impl Deref for ServerName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.original
    }
}
impl AsRef<str> for ServerName {
    fn as_ref(&self) -> &str {
        &self.original
    }
}
impl fmt::Display for ServerName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.original)
    }
}
impl From<&str> for ServerName {
    fn from(value: &str) -> Self {
        Self {
            original: value.to_owned(),
            folded: value.to_ascii_lowercase(),
        }
    }
}
impl From<String> for ServerName {
    fn from(value: String) -> Self {
        Self {
            folded: value.to_ascii_lowercase(),
            original: value,
        }
    }
}
impl From<&String> for ServerName {
    fn from(value: &String) -> Self {
        Self::from(value.as_str())
    }
}
impl From<&Self> for ServerName {
    fn from(value: &Self) -> Self {
        value.clone()
    }
}
#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};

    use super::ServerName;

    #[test]
    fn compares_ascii_case_insensitively() {
        let original = ServerName::from("irc.Example.NET");
        let folded = ServerName::from("IRC.example.net");

        assert_eq!(original, folded);
        assert_eq!(original, "IRC.EXAMPLE.NET");
        assert_eq!(original.to_string(), "irc.Example.NET");
        assert_eq!(HashSet::from([original.clone(), folded.clone()]).len(), 1);
        assert_eq!(BTreeSet::from([original, folded]).len(), 1);
    }

    #[test]
    fn builds_from_owned_and_borrowed_text() {
        let text = String::from("irc.example.net");
        let owned = ServerName::from(text.clone());

        assert_eq!(owned, ServerName::from(&text));
        assert_eq!(owned, ServerName::from(&owned));
    }
}
