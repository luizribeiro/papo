use super::Casemapping;
use crate::{AccountName, ChannelName, Mask, Nick, Origin};

impl Casemapping {
    /// Creates a nickname carrying these server-defined comparison rules.
    #[doc(hidden)]
    #[must_use]
    pub fn nick(self, text: impl Into<String>) -> Nick {
        Origin::NONE.__nick(self, text)
    }

    /// Creates a channel name carrying these server-defined comparison rules.
    #[doc(hidden)]
    #[must_use]
    pub fn channel(self, text: impl Into<String>) -> ChannelName {
        Origin::NONE.__channel(self, text)
    }

    /// Creates an account name carrying these server-defined comparison rules.
    #[doc(hidden)]
    #[must_use]
    pub fn account(self, text: impl Into<String>) -> AccountName {
        Origin::NONE.__account(self, text)
    }

    /// Creates a mask carrying these server-defined comparison rules.
    #[doc(hidden)]
    #[must_use]
    pub fn mask(self, text: impl Into<String>) -> Mask {
        Origin::NONE.__mask(self, text)
    }
}
