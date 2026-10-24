use std::{borrow::Cow, sync::Arc};

mod sealed {
    pub trait Sealed {}
}

/// An IRC address accepted by client operations.
///
/// Plain text is interpreted for the receiving client. Typed names and conversations retain the
/// client that produced them and are refused by another client.
pub trait Target: sealed::Sealed {
    /// Returns the wire text and its client identity.
    #[doc(hidden)]
    fn __target(&self) -> (&str, crate::Origin);
}

macro_rules! text_targets {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl sealed::Sealed for $ty {}
            impl Target for $ty {
                fn __target(&self) -> (&str, crate::Origin) {
                    (self.as_ref(), crate::Origin::NONE)
                }
            }
        )+
    };
}

text_targets!(str, String, Box<str>, Cow<'_, str>, Arc<str>);

macro_rules! name_targets {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl sealed::Sealed for $ty {}
            impl Target for $ty {
                fn __target(&self) -> (&str, crate::Origin) {
                    (self.as_ref(), self.origin())
                }
            }
        )+
    };
}

name_targets!(crate::Nick, crate::ChannelName, crate::Mask);

impl<T: Target + ?Sized> sealed::Sealed for &T {}

impl<T: Target + ?Sized> Target for &T {
    fn __target(&self) -> (&str, crate::Origin) {
        (*self).__target()
    }
}

impl sealed::Sealed for crate::Conversation {}

impl Target for crate::Conversation {
    fn __target(&self) -> (&str, crate::Origin) {
        match self {
            Self::Channel(channel) => channel.__target(),
            Self::Private(nick) => nick.__target(),
            Self::Broadcast(mask) => mask.__target(),
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
            crate::Conversation::Broadcast(mask.clone()),
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
