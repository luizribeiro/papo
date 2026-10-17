use crate::{ChannelName, ModeKind, Nick, Target};

use super::Source;

/// A user joined a channel shared with the client.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Joined {
    /// The joined channel.
    pub channel: ChannelName,
    /// The joining user's nickname.
    pub nick: Nick,
    /// Whether the joining user is this client.
    pub is_self: bool,
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
    /// Whether the departing user is this client.
    pub is_self: bool,
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
    pub by: Source,
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
    pub by: Source,
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
    pub by: Source,
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
    /// The mode target.
    pub target: Target,
    /// The user or server that changed the modes.
    pub by: Source,
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
    /// The server-supplied reason, which may be empty.
    pub reason: String,
}
