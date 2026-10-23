use std::{borrow::Borrow, cmp::Ordering};

use papo_proto::{AccountName, Casemapping, ChannelName, Mask, Nick, Origin};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Names {
    casemapping: Casemapping,
    origin: Origin,
}

impl Default for Names {
    fn default() -> Self {
        Self::new(Casemapping::default(), Origin::NONE)
    }
}

#[allow(dead_code)]
impl Names {
    pub(crate) const fn new(casemapping: Casemapping, origin: Origin) -> Self {
        Self {
            casemapping,
            origin,
        }
    }

    pub(crate) const fn casemapping(self) -> Casemapping {
        self.casemapping
    }

    pub(crate) const fn origin(self) -> Origin {
        self.origin
    }

    pub(crate) fn nick(self, text: impl AsRef<str>) -> Nick {
        self.origin.__nick(self.casemapping, text)
    }

    pub(crate) fn channel(self, text: impl AsRef<str>) -> ChannelName {
        self.origin.__channel(self.casemapping, text)
    }

    pub(crate) fn account(self, text: impl AsRef<str>) -> AccountName {
        self.origin.__account(self.casemapping, text)
    }

    pub(crate) fn mask(self, text: impl AsRef<str>) -> Mask {
        self.origin.__mask(self.casemapping, text)
    }
}

impl std::ops::Deref for Names {
    type Target = Casemapping;

    fn deref(&self) -> &Self::Target {
        &self.casemapping
    }
}

impl From<Names> for Casemapping {
    fn from(names: Names) -> Self {
        names.casemapping
    }
}

pub(crate) fn names_equal(casemapping: impl Into<Casemapping>, left: &str, right: &str) -> bool {
    let casemapping = casemapping.into();
    casemapping.fold(left) == casemapping.fold(right)
}

#[derive(Clone, Debug)]
pub(crate) struct NameKey<T> {
    original: T,
    folded: String,
}

impl<T: AsRef<str>> NameKey<T> {
    pub(crate) fn new(value: impl Into<T>, casemapping: impl Into<Casemapping>) -> Self {
        let original = value.into();
        let folded = casemapping.into().fold(original.as_ref()).into_owned();
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
