use std::fmt;

/// The IRC conversation to which a chat line belongs.
///
/// A channel handle retains a `STATUSMSG` audience when the received line had one. A conversation
/// is not a send target; answer through the line's return address so an ops-only audience cannot
/// be widened accidentally.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Conversation {
    /// Messages addressed to a channel.
    Channel(crate::ChannelHandle),
    /// Messages exchanged with the named other party.
    Private(crate::Nick),
}

impl Conversation {
    pub(crate) fn from_core(
        value: papo_core::Conversation,
        status: Option<crate::Prefix>,
        origin: crate::Origin,
        state: Option<&std::sync::Arc<std::sync::RwLock<std::sync::Arc<crate::State>>>>,
        guard: Option<papo_core::ChannelGuard>,
    ) -> Self {
        match value {
            papo_core::Conversation::Channel(channel) => {
                Self::Channel(crate::ChannelHandle::from_parts(
                    origin.__channel(channel.casemapping(), channel.into_string()),
                    status,
                    guard,
                    state,
                ))
            }
            papo_core::Conversation::Private(nick) => {
                Self::Private(origin.__nick(nick.casemapping(), nick.into_string()))
            }
            _ => unreachable!("core conversations have channel and private variants"),
        }
    }
}

pub(crate) fn public_from_core(
    value: papo_core::Conversation,
    _status: Option<crate::Prefix>,
    origin: crate::Origin,
    _state: Option<&std::sync::Arc<std::sync::RwLock<std::sync::Arc<crate::State>>>>,
    _guard: Option<papo_core::ChannelGuard>,
) -> crate::PublicConversation {
    match value {
        papo_core::Conversation::Channel(channel) => crate::PublicConversation::Channel(
            origin.__channel(channel.casemapping(), channel.into_string()),
        ),
        papo_core::Conversation::Private(nick) => crate::PublicConversation::Private(
            origin.__nick(nick.casemapping(), nick.into_string()),
        ),
        _ => unreachable!("core conversations have channel and private variants"),
    }
}

pub(crate) fn public_origin(value: &crate::PublicConversation) -> crate::Origin {
    match value {
        crate::PublicConversation::Channel(channel) => channel.origin(),
        crate::PublicConversation::Private(nick) => nick.origin(),
        _ => crate::Origin::NONE,
    }
}

pub(crate) fn public_private(value: &crate::PublicConversation) -> Option<&crate::Nick> {
    match value {
        crate::PublicConversation::Private(nick) => Some(nick),
        _ => None,
    }
}

pub(crate) fn public_channel_handle(
    _value: &crate::PublicConversation,
) -> Option<crate::ChannelHandle> {
    None
}

pub(crate) fn public_channel_name(value: &crate::PublicConversation) -> Option<crate::ChannelName> {
    match value {
        crate::PublicConversation::Channel(channel) => Some(channel.clone()),
        _ => None,
    }
}

pub(crate) fn public_channel(channel: &crate::ChannelHandle) -> crate::PublicConversation {
    crate::PublicConversation::Channel(channel.name())
}

pub(crate) fn public_to_handle(
    value: &crate::PublicConversation,
    origin: crate::Origin,
    status: Option<crate::Prefix>,
    guard: Option<papo_core::ChannelGuard>,
) -> Conversation {
    match value {
        crate::PublicConversation::Channel(channel) => {
            Conversation::Channel(crate::ChannelHandle::from_parts(
                origin.__channel(channel.casemapping(), channel.as_str()),
                status,
                guard,
                None,
            ))
        }
        crate::PublicConversation::Private(nick) => {
            Conversation::Private(origin.__nick(nick.casemapping(), nick.as_str()))
        }
        _ => unreachable!("core conversations have channel and private variants"),
    }
}

impl fmt::Display for Conversation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Channel(channel) => channel.name().fmt(formatter),
            Self::Private(nick) => nick.fmt(formatter),
        }
    }
}

impl PartialEq<str> for Conversation {
    fn eq(&self, other: &str) -> bool {
        match self {
            Self::Channel(channel) => channel.name() == other,
            Self::Private(nick) => nick == other,
        }
    }
}

impl PartialEq<&str> for Conversation {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}
