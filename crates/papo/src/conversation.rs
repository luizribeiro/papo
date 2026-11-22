use std::fmt;

/// The IRC conversation to which a chat line belongs: the window an application files it in.
///
/// # Ops-only lines
///
/// A line sent to a status-prefixed address such as `@#ops` reached only the channel's operators
/// (`+#ops`: voiced members and up). Its conversation is still the plain channel `#ops`, so an
/// ops-only line and a public line in `#ops` have equal conversations and share one buffer,
/// history, read marker and member list. The audience stays with the line: such a line arrives
/// only in [`crate::Event::Restricted`] (live, and among [`crate::HistoryPage::items`]), whose
/// `audience` names it. Answer it with [`crate::Client::reply`], [`crate::Client::react`],
/// [`crate::Client::redact`] or the line's `return_address()`, which keep `@#ops`.
///
/// Code that never matches the restricted arm never sees ops-only lines, so a bot that announces
/// to `#ops` from its message handler cannot repeat an ops-only command to everyone. Its cost is
/// the other direction: such code misses those lines entirely, and a log or a window that should
/// show them has to handle the arm. State still counts them under the plain channel: the typing
/// list shows a typer who typed to `@#ops` (each [`crate::Typer`] keeps that address), and
/// [`crate::Client::mark_read`] without an explicit boundary marks through the newest line,
/// restricted or not. Ergo stores no ops-only lines in history; soju stores and replays them.
///
/// To send to everyone in `#ops`, pass the plain name `"#ops"` or the conversation's channel handle.
///
/// A server can also hide an audience without a sign (Ergo's `+U`, Solanum's `+z`): such a line
/// arrives as plain `#ops` and no client can tell.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Conversation {
    /// Messages addressed to a channel.
    Channel(crate::ChannelHandle),
    /// Messages exchanged with a user. Equality follows [`crate::UserHandle`].
    Private(crate::UserHandle),
}

impl Conversation {
    pub(crate) fn from_core_with_state(
        value: papo_core::Conversation,
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
        Self::from_core(value, origin, Some(state), guard)
    }

    /// Builds a live line's conversation; a private peer comes from the identity recorded
    /// when the line arrived.
    pub(crate) fn at_arrival(
        value: papo_core::Conversation,
        origin: crate::Origin,
        state: Option<&std::sync::Arc<std::sync::RwLock<std::sync::Arc<crate::State>>>>,
        guard: Option<papo_core::ChannelGuard>,
        peer_token: papo_core::ReplyToken,
    ) -> Self {
        match value {
            papo_core::Conversation::Private(nick) => Self::Private(crate::UserHandle::at_arrival(
                origin.__nick(nick.casemapping(), nick.into_string()),
                crate::Account::Unknown,
                peer_token,
                state,
            )),
            channel => Self::from_core(channel, origin, state, guard),
        }
    }

    pub(crate) fn from_core(
        value: papo_core::Conversation,
        origin: crate::Origin,
        state: Option<&std::sync::Arc<std::sync::RwLock<std::sync::Arc<crate::State>>>>,
        guard: Option<papo_core::ChannelGuard>,
    ) -> Self {
        match value {
            papo_core::Conversation::Channel(channel) => {
                Self::Channel(crate::ChannelHandle::from_parts(
                    origin.__channel(channel.casemapping(), channel.into_string()),
                    guard,
                    state,
                ))
            }
            papo_core::Conversation::Private(nick) => {
                Self::Private(crate::UserHandle::from_current_state(
                    origin.__nick(nick.casemapping(), nick.into_string()),
                    state,
                ))
            }
            _ => unreachable!("core conversations have channel and private variants"),
        }
    }
}

impl fmt::Display for Conversation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Channel(channel) => channel.name().fmt(formatter),
            Self::Private(user) => user.nick().fmt(formatter),
        }
    }
}

impl PartialEq<str> for Conversation {
    fn eq(&self, other: &str) -> bool {
        match self {
            Self::Channel(channel) => channel.name() == other,
            Self::Private(user) => user.nick() == other,
        }
    }
}

impl PartialEq<&str> for Conversation {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}
