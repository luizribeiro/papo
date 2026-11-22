use std::fmt;

/// The IRC conversation to which a chat line belongs: the window an application files it in.
///
/// # Ops-only lines
///
/// A line sent to a status-prefixed address such as `@#ops` reached only the channel's operators
/// (`+#ops`: voiced members and up). Its conversation is still the plain channel `#ops`, so an
/// ops-only line and a public line in `#ops` have equal conversations and share one buffer,
/// history, read marker and member list. The audience stays with the line: such a line arrives
/// only in [`crate::Event::Restricted`] (live, and among [`crate::event::HistoryPage::items`]), whose
/// `audience` names it. Answer it with [`crate::Client::reply`], [`crate::Client::react`],
/// [`crate::Client::redact`] or the line's `return_address()`, which keep `@#ops`.
///
/// Match [`crate::Event::Restricted`] and its [`crate::event::RestrictedLine`] to display or act on
/// ops-only lines. The typing list still files them under the plain channel. Read the prefix with
/// [`crate::Typer::audience`] or use [`crate::Typer::return_address`] to keep the audience.
/// [`crate::Client::mark_read`] without an explicit boundary marks through the newest line,
/// including restricted lines. Ergo stores no ops-only lines in history. Soju can replay them.
///
/// A [`crate::ChannelRef`] names the whole channel. To address everyone, send to that reference
/// or use the plain name `"#ops"`.
///
/// A server can also hide an audience without a sign (Ergo's `+U`, Solanum's `+z`): such a line
/// arrives as plain `#ops` and no client can tell.
///
/// ```compile_fail,E0308
/// fn matches_text(reference: papo::Conversation) -> bool {
///     reference == "#rust"
/// }
/// ```
///
/// ```compile_fail,E0277
/// fn matches_text(reference: &papo::Conversation) -> bool {
///     reference == "#rust"
/// }
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Conversation {
    /// Messages addressed to a channel.
    Channel(crate::ChannelRef),
    /// Messages exchanged with a user. Equality follows [`crate::UserRef`].
    Private(crate::UserRef),
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
            papo_core::Conversation::Private(nick) => Self::Private(crate::UserRef::at_arrival(
                origin.__nick(nick.casemapping(), nick.into_string()),
                crate::AccountStatus::Unknown,
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
                Self::Channel(crate::ChannelRef::from_parts(
                    origin.__channel(channel.casemapping(), channel.into_string()),
                    guard,
                    state,
                ))
            }
            papo_core::Conversation::Private(nick) => {
                Self::Private(crate::UserRef::from_current_state(
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
