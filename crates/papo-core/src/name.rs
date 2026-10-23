use std::{borrow::Borrow, cmp::Ordering};

use papo_proto::Casemapping;

pub(crate) fn names_equal(casemapping: Casemapping, left: &str, right: &str) -> bool {
    casemapping.fold(left) == casemapping.fold(right)
}

#[derive(Clone, Debug)]
pub(crate) struct NameKey<T> {
    original: T,
    folded: String,
}

impl<T: AsRef<str>> NameKey<T> {
    pub(crate) fn new(value: impl Into<T>, casemapping: Casemapping) -> Self {
        let original = value.into();
        let folded = casemapping.fold(original.as_ref()).into_owned();
        Self { original, folded }
    }

    pub(crate) fn original(&self) -> &str {
        self.original.as_ref()
    }
}

impl<T> PartialEq for NameKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.folded == other.folded
    }
}

impl<T> Eq for NameKey<T> {}

impl<T> PartialOrd for NameKey<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for NameKey<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.folded.cmp(&other.folded)
    }
}

impl<T> Borrow<str> for NameKey<T> {
    fn borrow(&self) -> &str {
        &self.folded
    }
}
