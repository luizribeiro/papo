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
    pub(crate) fn from_core_with_state(
        value: papo_core::Conversation,
        status: Option<crate::Prefix>,
        origin: crate::Origin,
        state: &std::sync::Arc<std::sync::RwLock<std::sync::Arc<crate::State>>>,
        snapshot: &crate::State,
    ) -> Self {
        let guard = match &value {
            papo_core::Conversation::Channel(channel) => {
                Some(snapshot.core.__channel_guard(channel))
            }
            _ => None,
        };
        Self::from_core(value, status, origin, Some(state), guard)
    }

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
