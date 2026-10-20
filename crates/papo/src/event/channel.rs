use crate::{Authorship, ChannelName, Nick, Subject};

use super::Source;

/// The parameter behavior of a channel mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ModeKind {
    /// A list mode that always takes an argument.
    List,
    /// A mode that always takes an argument.
    AlwaysArg,
    /// A mode that takes an argument only when being set.
    ArgWhenSet,
    /// A mode that never takes an argument.
    NoArg,
    /// A channel-membership mode with its zero-based privilege rank.
    Prefix(usize),
    /// A mode whose parameter behavior was not advertised.
    Unknown,
}

impl ModeKind {
    pub(crate) const fn from_proto(kind: papo_proto::ModeKind) -> Self {
        match kind {
            papo_proto::ModeKind::List => Self::List,
            papo_proto::ModeKind::AlwaysArg => Self::AlwaysArg,
            papo_proto::ModeKind::ArgWhenSet => Self::ArgWhenSet,
            papo_proto::ModeKind::NoArg => Self::NoArg,
            papo_proto::ModeKind::Prefix(rank) => Self::Prefix(rank),
            papo_proto::ModeKind::Unknown => Self::Unknown,
        }
    }
}

/// A user joined a channel shared with the client.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Joined {
    /// The joined channel.
    pub channel: ChannelName,
    /// The joining user's nickname.
    pub nick: Nick,
    /// Whether the joining user is this client's user.
    pub authorship: Authorship,
}

/// Initial channel membership synchronization completed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ChannelSynced {
    /// The synchronized channel.
    pub channel: ChannelName,
}

/// A user left a shared channel with `PART`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Parted {
    /// The departed channel.
    pub channel: ChannelName,
    /// The departing user's nickname.
    pub nick: Nick,
    /// The optional part reason.
    pub reason: Option<String>,
    /// Whether the departing user is this client's user.
    pub authorship: Authorship,
}

/// A user was removed from a shared channel with `KICK`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Kicked {
    /// The affected channel.
    pub channel: ChannelName,
    /// The removed user's nickname.
    pub nick: Nick,
    /// The user or server that issued the kick.
    pub actor: Source,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// Whether the removed user is this client's user.
    pub subject_authorship: Authorship,
    /// The optional kick reason.
    pub reason: Option<String>,
    /// Whether the removed user is this client.
    pub is_self: bool,
}

/// A user was invited to a channel.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Invited {
    /// The invited channel.
    pub channel: ChannelName,
    /// The invited user's nickname.
    pub nick: Nick,
    /// The user or server that sent the invitation.
    pub actor: Source,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// Whether the invited user is this client's user.
    pub subject_authorship: Authorship,
    /// Whether the invited user is this client.
    pub is_self: bool,
}

/// A channel topic changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TopicChanged {
    /// The affected channel.
    pub channel: ChannelName,
    /// The new topic, or `None` when it was cleared.
    pub topic: Option<String>,
    /// The user or server that changed the topic.
    pub actor: Source,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
}

/// An owned operation from an IRC mode string.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ModeChange {
    /// Whether the mode was set rather than removed.
    pub set: bool,
    /// The mode letter.
    pub mode: char,
    /// The mode's advertised parameter behavior.
    pub kind: ModeKind,
    /// The argument required by [`Self::kind`], when supplied by the server.
    pub arg: Option<String>,
}

/// A user or channel mode changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ModeChanged {
    /// The user or channel whose modes changed.
    pub subject: Subject,
    /// The user or server that changed the modes.
    pub actor: Source,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// Whether the distinct subject is this client's user.
    pub subject_authorship: Authorship,
    /// Parsed mode operations in wire order.
    pub changes: Vec<ModeChange>,
}

/// A joined channel was renamed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ChannelRenamed {
    /// The previous channel name.
    pub old: ChannelName,
    /// The new channel name.
    pub new: ChannelName,
    /// The user or server that renamed the channel.
    pub actor: Source,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// The server-supplied reason, when present and non-empty.
    pub reason: Option<String>,
}
