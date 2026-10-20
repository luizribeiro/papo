use std::{
    borrow::Borrow,
    hash::{Hash, Hasher},
};

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

impl<T> Hash for NameKey<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.folded.hash(state);
    }
}

impl<T> Borrow<str> for NameKey<T> {
    fn borrow(&self) -> &str {
        &self.folded
    }
}
