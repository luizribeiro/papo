use std::{collections::btree_map, iter::Map};

use crate::Cap;

/// Enabled `IRCv3` capabilities and the values advertised by the server.
///
/// Lookups use [`Cap`] identity, so known draft and final spellings match.
/// Iteration preserves the spelling negotiated with the server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capabilities(pub(crate) papo_core::Capabilities);

impl Capabilities {
    /// Returns the capability's advertised value.
    ///
    /// `None` means the capability is absent. `Some(None)` means it was enabled
    /// without a value, and `Some(Some(value))` holds the value verbatim.
    #[must_use]
    pub fn get(&self, cap: &Cap) -> Option<Option<&str>> {
        self.0.get(cap).map(Option::as_deref)
    }

    /// Reports whether a capability is present, including entries without a value.
    #[must_use]
    pub fn contains(&self, cap: &Cap) -> bool {
        self.0.contains_key(cap)
    }

    /// Iterates over capabilities in canonical name order, with their optional advertised values.
    #[must_use]
    pub fn iter(
        &self,
    ) -> impl ExactSizeIterator<Item = (&Cap, Option<&str>)> + DoubleEndedIterator {
        self.into_iter()
    }
}

impl<'a> IntoIterator for &'a Capabilities {
    type Item = (&'a Cap, Option<&'a str>);
    type IntoIter = Map<
        btree_map::Iter<'a, Cap, Option<String>>,
        fn((&'a Cap, &'a Option<String>)) -> Self::Item,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().map(|(name, value)| (name, value.as_deref()))
    }
}
