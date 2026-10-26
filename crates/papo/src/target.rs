use std::{borrow::Cow, sync::Arc};

pub(crate) mod sealed {
    use std::borrow::Cow;

    pub trait Sealed {}

    #[derive(Clone)]
    pub struct Parts<'a> {
        pub(crate) text: Cow<'a, str>,
        pub(crate) origin: crate::Origin,
        pub(crate) recipient: Option<crate::message_ref::RecipientGuard>,
    }

    impl Parts<'_> {
        pub(crate) fn into_owned(self) -> Parts<'static> {
            Parts {
                text: Cow::Owned(self.text.into_owned()),
                origin: self.origin,
                recipient: self.recipient,
            }
        }
    }
}

/// An IRC address accepted by client operations.
///
/// Plain text is interpreted for the receiving client. Typed names and conversations retain the
/// client that produced them and are refused by another client.
pub trait Target: sealed::Sealed {
    /// Returns the destination and its safety metadata.
    #[doc(hidden)]
    fn __parts(&self) -> sealed::Parts<'_>;
}

/// A channel-name argument accepted by channel operations.
///
/// Text is interpreted using the receiving client's current `ISUPPORT` rules. A typed
/// [`crate::ChannelName`] retains the client that produced it.
pub trait AsChannel: sealed::Sealed {
    /// Returns the channel name and its safety metadata.
    #[doc(hidden)]
    fn __parts(&self) -> sealed::Parts<'_>;
}

/// A nickname argument accepted by nickname operations.
///
/// Text is interpreted using the receiving client's current `ISUPPORT` rules. A typed
/// [`crate::Nick`] retains the client that produced it.
pub trait AsNick: sealed::Sealed {
    /// Returns the nickname and its safety metadata.
    #[doc(hidden)]
    fn __parts(&self) -> sealed::Parts<'_>;
}

/// A channel or private-conversation argument.
///
/// Text is interpreted using the receiving client's current `ISUPPORT` rules. Status-prefixed
/// addresses such as `@#ops` are not conversations; see [`crate::Conversation`] for guidance.
pub trait AsConversation: sealed::Sealed {
    /// Returns the conversation name and its safety metadata.
    #[doc(hidden)]
    fn __parts(&self) -> sealed::Parts<'_>;
}

fn text_parts(text: &str) -> sealed::Parts<'_> {
    sealed::Parts {
        text: Cow::Borrowed(text),
        origin: crate::Origin::NONE,
        recipient: None,
    }
}

macro_rules! text_roles {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl sealed::Sealed for $ty {}

            impl Target for $ty {
                fn __parts(&self) -> sealed::Parts<'_> {
                    text_parts(self.as_ref())
                }
            }

            impl AsChannel for $ty {
                fn __parts(&self) -> sealed::Parts<'_> {
                    text_parts(self.as_ref())
                }
            }

            impl AsNick for $ty {
                fn __parts(&self) -> sealed::Parts<'_> {
                    text_parts(self.as_ref())
                }
            }

            impl AsConversation for $ty {
                fn __parts(&self) -> sealed::Parts<'_> {
                    text_parts(self.as_ref())
                }
            }
        )+
    };
}

text_roles!(str, String, Box<str>, Cow<'_, str>, Arc<str>);

fn name_parts(text: &str, origin: crate::Origin) -> sealed::Parts<'_> {
    sealed::Parts {
        text: Cow::Borrowed(text),
        origin,
        recipient: None,
    }
}

impl sealed::Sealed for crate::Nick {}

impl Target for crate::Nick {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_ref(), self.origin())
    }
}

impl AsNick for crate::Nick {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_ref(), self.origin())
    }
}

impl AsConversation for crate::Nick {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_ref(), self.origin())
    }
}

impl sealed::Sealed for crate::ChannelName {}

impl Target for crate::ChannelName {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_ref(), self.origin())
    }
}

impl AsChannel for crate::ChannelName {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_ref(), self.origin())
    }
}

impl AsConversation for crate::ChannelName {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_ref(), self.origin())
    }
}

impl sealed::Sealed for crate::Conversation {}

impl Target for crate::Conversation {
    fn __target(&self) -> (&str, crate::Origin) {
        match self {
            Self::Channel(channel) => channel.__target(),
            Self::Private(nick) => nick.__target(),
            _ => ("*", crate::Origin::NONE),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Target;

    #[test]
    fn every_target_kind_reports_text_and_origin() {
        let origin = crate::Origin::__new(19);
        let mapping = crate::Casemapping::default();
        let channel = origin.__channel(mapping, "#papo");
        let nick = origin.__nick(mapping, "emily");
        let mask = origin.__mask(mapping, "$*.example.org");
        for target in [
            crate::Conversation::Channel(channel.clone()),
            crate::Conversation::Private(nick.clone()),
        ] {
            assert_eq!(target.__target().1, origin);
        }
        assert_eq!(
            crate::Conversation::Channel(channel).__target(),
            ("#papo", origin)
        );
        assert_eq!(
            crate::Conversation::Private(nick).__target(),
            ("emily", origin)
        );
        assert_eq!(mask.__target(), ("$*.example.org", origin));
        assert_eq!("#papo".__target(), ("#papo", crate::Origin::NONE));
    }
}
