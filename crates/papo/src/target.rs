use std::{borrow::Cow, sync::Arc};

pub(crate) mod sealed {
    use std::borrow::Cow;

    pub trait Sealed {}

    #[derive(Clone)]
    pub struct Parts<'a> {
        pub(crate) text: Cow<'a, str>,
        pub(crate) origin: crate::Origin,
        pub(crate) recipient: Option<crate::message_ref::RecipientGuard>,
        pub(crate) channel: Option<Box<papo_core::ChannelGuard>>,
    }

    impl Parts<'_> {
        pub(crate) fn into_owned(self) -> Parts<'static> {
            Parts {
                text: Cow::Owned(self.text.into_owned()),
                origin: self.origin,
                recipient: self.recipient,
                channel: self.channel,
            }
        }
    }

    impl<'a> From<papo_proto::NameParts<'a>> for Parts<'a> {
        fn from(parts: papo_proto::NameParts<'a>) -> Self {
            Self {
                text: Cow::Owned(parts.text().to_owned()),
                origin: parts.origin(),
                recipient: None,
                channel: None,
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Role {
    Send(&'static str),
    Channel(&'static str),
    Nick(&'static str),
    Account(&'static str),
    Conversation(&'static str),
    Mask(&'static str),
}

impl Role {
    pub(crate) const fn argument(self) -> &'static str {
        match self {
            Self::Send(argument)
            | Self::Channel(argument)
            | Self::Nick(argument)
            | Self::Account(argument)
            | Self::Conversation(argument)
            | Self::Mask(argument) => argument,
        }
    }
}

/// A destination accepted by message-sending operations.
///
/// Plain text is accepted only when it names a channel. Direct user lines require a
/// [`crate::DirectRef`]. Typed channel names and message references retain the client that
/// produced them and are refused by another client. Status prefixes such as `@#ops` are
/// preserved. [`crate::Client::send_raw`] is the explicit escape hatch for unguarded targets and
/// server-wide targets such as `*` and `$opers`.
pub trait Target: sealed::Sealed {
    /// Returns the destination and its safety metadata.
    #[doc(hidden)]
    fn __parts(&self) -> sealed::Parts<'_>;
}

/// A nickname accepted by user-oriented operations.
///
/// A [`crate::DirectRef`] retains the recipient identity so operations can refuse a nickname
/// handoff. Plain text and [`crate::Nick`] identify whoever currently holds that nickname.
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

/// A mask, nickname, or channel accepted by [`crate::Client::who`].
///
/// Text is interpreted using the receiving client's current `ISUPPORT` rules. A typed value
/// retains the client that produced it. Status-prefixed addresses such as `@#ops` are refused.
pub trait AsMask: sealed::Sealed {
    /// Returns the mask and its safety metadata.
    #[doc(hidden)]
    fn __parts(&self) -> sealed::Parts<'_>;
}

/// An IRC capability name accepted by client configuration.
///
/// Plain text and typed [`crate::Cap`] values are accepted. Typed constants are available in
/// [`crate::cap`].
pub trait AsCap: sealed::Sealed {
    /// Returns the owned capability name.
    #[doc(hidden)]
    fn __cap(&self) -> crate::Cap;
}

fn text_parts(text: &str) -> sealed::Parts<'_> {
    sealed::Parts {
        text: Cow::Borrowed(text),
        origin: crate::Origin::NONE,
        recipient: None,
        channel: None,
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

            impl AsCap for $ty {
                fn __cap(&self) -> crate::Cap {
                    let text: &str = self.as_ref();
                    crate::Cap::from(text)
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
        channel: None,
    }
}

impl sealed::Sealed for crate::Nick {}

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

impl AsMask for crate::Nick {
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

impl AsConversation for crate::ChannelName {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_ref(), self.origin())
    }
}

impl AsMask for crate::ChannelName {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_ref(), self.origin())
    }
}

impl sealed::Sealed for crate::Mask {}

impl AsMask for crate::Mask {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_ref(), self.origin())
    }
}

impl sealed::Sealed for crate::Cap {}

impl AsCap for crate::Cap {
    fn __cap(&self) -> crate::Cap {
        self.clone()
    }
}

impl sealed::Sealed for crate::Conversation {}

impl AsConversation for crate::Conversation {
    fn __parts(&self) -> sealed::Parts<'_> {
        match self {
            Self::Channel(channel) => AsConversation::__parts(channel),
            Self::Private(nick) => AsConversation::__parts(nick),
            _ => text_parts("*"),
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
reference_role!(AsConversation);
reference_role!(AsMask);

impl<T: AsCap + ?Sized> AsCap for &T {
    fn __cap(&self) -> crate::Cap {
        <T as AsCap>::__cap(*self)
    }
}

#[cfg(test)]
mod tests {
    use super::{AsCap, AsConversation, AsMask, Target};
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
        assert_eq!(
            AsCap::__cap(&crate::cap::CHATHISTORY),
            "draft/chathistory".into()
        );
        assert_eq!(AsCap::__cap(&"chathistory"), "chathistory".into());
    }
}
