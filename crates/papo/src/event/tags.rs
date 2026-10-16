use std::fmt;

#[cfg(test)]
use papo_proto::MessageRef;

/// Owned IRC message tags.
///
/// Tags iterate in wire order. Lookup uses the last occurrence of a key, and
/// equality compares those effective values without considering wire order.
#[derive(Clone, Default)]
pub struct Tags(Vec<(String, Option<String>)>);

impl Tags {
    /// Returns the last value for the case-sensitive `key`.
    ///
    /// A tag without a value returns `Some("")`. `None` means the key is absent.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.as_deref().unwrap_or(""))
    }

    /// Reports whether `key` occurs in these tags.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.0.iter().any(|(candidate, _)| candidate == key)
    }

    /// Iterates over tag keys and values in wire order.
    ///
    /// A valueless tag has a `None` value. Duplicate keys remain visible.
    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, Option<&str>)> {
        self.0
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_deref()))
    }

    /// Reports whether no tags were supplied.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn from_message(message: &MessageRef<'_>) -> Self {
        Self(
            message
                .tags()
                .map(|(key, value)| (key.to_owned(), value.map(std::borrow::Cow::into_owned)))
                .collect(),
        )
    }
}

impl fmt::Debug for Tags {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(&self.0).finish()
    }
}

impl PartialEq for Tags {
    fn eq(&self, other: &Self) -> bool {
        self.0
            .iter()
            .all(|(key, _)| self.get(key) == other.get(key))
            && other
                .0
                .iter()
                .all(|(key, _)| self.get(key) == other.get(key))
    }
}

impl Eq for Tags {}

#[cfg(test)]
mod tests {
    use papo_proto::MessageRef;

    use super::Tags;

    fn tags(line: &str) -> Tags {
        Tags::from_message(&MessageRef::parse(line).unwrap())
    }

    #[test]
    fn unescapes_values_and_resolves_duplicates_last_wins() {
        let tags = tags(
            "@account=alice;draft/label=old;bot;example.org/note=one\\stwo\\:three;draft/label=new :alice!~a@cloak.example PRIVMSG #papo :release ready",
        );

        assert_eq!(tags.get("account"), Some("alice"));
        assert_eq!(tags.get("draft/label"), Some("new"));
        assert_eq!(tags.get("bot"), Some(""));
        assert_eq!(tags.get("example.org/note"), Some("one two;three"));
        assert_eq!(tags.get("missing"), None);
        assert!(tags.contains("bot"));
        assert_eq!(tags.iter().count(), 5);
        assert_eq!(tags.iter().nth(1), Some(("draft/label", Some("old"))));
    }

    #[test]
    fn equality_uses_effective_values_without_wire_order() {
        let left = tags("@account=old;bot;account=alice :ergo.test TAGMSG #papo");
        let reordered = tags("@account=alice;bot :ergo.test TAGMSG #papo");
        let different = tags("@account=bob;bot :ergo.test TAGMSG #papo");

        assert_eq!(left, reordered);
        assert_ne!(left, different);
    }
}
