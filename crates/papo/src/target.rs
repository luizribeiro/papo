use std::{borrow::Cow, sync::Arc};

pub(crate) mod sealed {
    use std::borrow::Cow;

    pub trait Sealed {}

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum SendName {
        Text,
        Nick,
        HistoryPeer,
    }

    #[derive(Clone)]
    pub struct Parts<'a> {
        pub(crate) text: Cow<'a, str>,
        pub(crate) origin: crate::Origin,
        pub(crate) send_name: Option<SendName>,
        pub(crate) channel: Option<Box<papo_core::ChannelGuard>>,
        pub(crate) user: Option<crate::UserRef>,
    }

    impl Parts<'_> {
        pub(crate) fn text(&self) -> &str {
            &self.text
        }

        pub(crate) fn into_owned(self) -> Parts<'static> {
            Parts {
                text: Cow::Owned(self.text.into_owned()),
                origin: self.origin,
                send_name: self.send_name,
                channel: self.channel,
                user: self.user,
            }
        }
    }

    impl<'a> From<papo_proto::NameParts<'a>> for Parts<'a> {
        fn from(parts: papo_proto::NameParts<'a>) -> Self {
            Self {
                text: Cow::Owned(parts.text().to_owned()),
                origin: parts.origin(),
                send_name: None,
                channel: None,
                user: None,
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Role {
    Channel(&'static str),
    Nick(&'static str),
    Account(&'static str),
    Conversation(&'static str),
    Mask(&'static str),
}

impl Role {
    pub(crate) const fn argument(self) -> &'static str {
        match self {
            Self::Channel(argument)
            | Self::Nick(argument)
            | Self::Account(argument)
            | Self::Conversation(argument)
            | Self::Mask(argument) => argument,
        }
    }
}

/// A destination accepted by message-sending operations.
///
/// Plain text names a channel if it starts with a server-advertised channel prefix. A nickname
/// in text or as a [`crate::Nick`] addresses whoever holds it. Typed names, user references, channel handles, and message
/// references retain the client that produced them and are refused by another client.
/// A single status prefix such as `@#ops` is preserved; stacked prefixes such as `+@#ops` are
/// refused because servers disagree on their audience. [`crate::Client::send_raw`] is the explicit
/// escape hatch for server-wide targets such as `*` and `$opers`. See [`crate::Conversation`] for
/// guidance on status-prefixed audiences.
pub trait Target: sealed::Sealed {
    /// Returns the destination and its safety metadata.
    #[doc(hidden)]
    fn __parts(&self) -> sealed::Parts<'_>;
}

/// A nickname accepted by user-oriented operations.
///
/// A [`crate::UserRef`] retains the recipient identity so operations can refuse a nickname
/// handoff.
/// Plain text and [`crate::Nick`] identify whoever currently holds that nickname.
pub trait AsNick: sealed::Sealed {
    /// Returns the nickname and its safety metadata.
    #[doc(hidden)]
    fn __parts(&self) -> sealed::Parts<'_>;
}

/// A channel accepted by channel-oriented operations.
///
/// Text identifies the channel currently using that name. Live channel handles retain the
/// channel lifecycle captured by the client.
pub trait AsChannel: sealed::Sealed {
    /// Returns the channel and its safety metadata.
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

/// A mask, nickname, or channel accepted by [`crate::Client::who`].
///
/// Text is interpreted using the receiving client's current `ISUPPORT` rules. A typed value
/// retains the client that produced it. Status-prefixed addresses such as `@#ops` are refused.
pub trait AsMask: sealed::Sealed {
    /// Returns the mask and its safety metadata.
    #[doc(hidden)]
    fn __parts(&self) -> sealed::Parts<'_>;
}

fn text_parts(text: &str) -> sealed::Parts<'_> {
    sealed::Parts {
        text: Cow::Borrowed(text),
        origin: crate::Origin::NONE,
        send_name: Some(sealed::SendName::Text),
        channel: None,
        user: None,
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

            impl AsNick for $ty {
                fn __parts(&self) -> sealed::Parts<'_> {
                    text_parts(self.as_ref())
                }
            }

            impl AsChannel for $ty {
                fn __parts(&self) -> sealed::Parts<'_> {
                    text_parts(self.as_ref())
                }
            }

            impl AsConversation for $ty {
                fn __parts(&self) -> sealed::Parts<'_> {
                    text_parts(self.as_ref())
                }
            }

            impl AsMask for $ty {
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
        send_name: None,
        channel: None,
        user: None,
    }
}

impl sealed::Sealed for crate::Nick {}

impl Target for crate::Nick {
    fn __parts(&self) -> sealed::Parts<'_> {
        sealed::Parts {
            send_name: Some(sealed::SendName::Nick),
            ..name_parts(self.as_str(), self.origin())
        }
    }
}

impl AsNick for crate::Nick {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_str(), self.origin())
    }
}

impl AsConversation for crate::Nick {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_str(), self.origin())
    }
}

impl AsMask for crate::Nick {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_str(), self.origin())
    }
}

impl sealed::Sealed for crate::ChannelName {}

impl AsChannel for crate::ChannelName {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_str(), self.origin())
    }
}

impl Target for crate::ChannelName {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_str(), self.origin())
    }
}

impl AsConversation for crate::ChannelName {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_str(), self.origin())
    }
}

impl AsMask for crate::ChannelName {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_str(), self.origin())
    }
}

impl sealed::Sealed for crate::Mask {}

impl AsMask for crate::Mask {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_str(), self.origin())
    }
}

impl sealed::Sealed for crate::Conversation {}

impl AsConversation for crate::Conversation {
    fn __parts(&self) -> sealed::Parts<'_> {
        match self {
            Self::Channel(channel) => AsConversation::__parts(channel),
            Self::Private(nick) => AsConversation::__parts(nick),
        }
    }
}

impl<T: sealed::Sealed + ?Sized> sealed::Sealed for &T {}

macro_rules! reference_role {
    ($trait:ident) => {
        impl<T: $trait + ?Sized> $trait for &T {
            fn __parts(&self) -> sealed::Parts<'_> {
                <T as $trait>::__parts(*self)
            }
        }
    };
}

reference_role!(Target);
reference_role!(AsNick);
reference_role!(AsChannel);
reference_role!(AsConversation);
reference_role!(AsMask);

#[cfg(test)]
mod tests {
    use super::{AsConversation, AsMask, Target};
    use crate::{AsChannel, AsNick};

    #[test]
    fn typed_roles_report_text_and_origin() {
        let origin = crate::Origin::__new(19);
        let mapping = crate::Casemapping::default();
        let channel = origin.__channel(mapping, "#papo");
        let nick = origin.__nick(mapping, "emily");
        let mask = origin.__mask(mapping, "*!*@gateway.example");

        assert_eq!(AsChannel::__parts(&channel).text(), "#papo");
        assert_eq!(AsConversation::__parts(&channel).origin, origin);
        assert_eq!(AsNick::__parts(&nick).text, "emily");
        assert_eq!(AsMask::__parts(&mask).text, "*!*@gateway.example");
        assert_eq!(AsMask::__parts(&mask).origin, origin);
        assert_eq!(Target::__parts(&"@#ops").text, "@#ops");
    }
}
