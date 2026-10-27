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

    impl<'a> From<papo_proto::NameParts<'a>> for Parts<'a> {
        fn from(parts: papo_proto::NameParts<'a>) -> Self {
            Self {
                text: Cow::Owned(parts.text().to_owned()),
                origin: parts.origin(),
                recipient: None,
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
    Any(&'static str),
}

impl Role {
    pub(crate) const fn argument(self) -> &'static str {
        match self {
            Self::Send(argument)
            | Self::Channel(argument)
            | Self::Nick(argument)
            | Self::Account(argument)
            | Self::Conversation(argument)
            | Self::Any(argument) => argument,
        }
    }
}

/// A destination accepted by message-sending operations.
///
/// Plain text is interpreted for the receiving client. Typed names, conversations, and message
/// references retain the client that produced them and are refused by another client. Status
/// prefixes such as `@#ops` are preserved. Server-wide targets such as `*` and `$opers` are not
/// accepted; use [`crate::Client::send_raw`] for operator commands that require them.
pub trait Target: sealed::Sealed {
    /// Returns the destination and its safety metadata.
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

impl AsConversation for crate::ChannelName {
    fn __parts(&self) -> sealed::Parts<'_> {
        name_parts(self.as_ref(), self.origin())
    }
}

impl sealed::Sealed for crate::Conversation {}

impl Target for crate::Conversation {
    fn __parts(&self) -> sealed::Parts<'_> {
        match self {
            Self::Channel(channel) => Target::__parts(channel),
            Self::Private(nick) => Target::__parts(nick),
            _ => text_parts("*"),
        }
    }
}

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
reference_role!(AsConversation);

#[cfg(test)]
mod tests {
    use super::{AsConversation, Target};
    use crate::{AsChannel, AsNick};

    #[test]
    fn typed_roles_report_text_and_origin() {
        let origin = crate::Origin::__new(19);
        let mapping = crate::Casemapping::default();
        let channel = origin.__channel(mapping, "#papo");
        let nick = origin.__nick(mapping, "emily");

        assert_eq!(AsChannel::__parts(&channel).text(), "#papo");
        assert_eq!(AsConversation::__parts(&channel).origin, origin);
        assert_eq!(AsNick::__parts(&nick).text(), "emily");
        assert_eq!(Target::__parts(&nick).origin, origin);
        assert_eq!(Target::__parts(&"@#ops").text, "@#ops");
    }
}
