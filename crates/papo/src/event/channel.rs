use std::time::SystemTime;

use crate::{Account, Authorship, ChannelName, MessageId, Nick, Subject};

use super::{Source, UserSource};

/// A user joined a channel shared with the client.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Joined {
    /// The server-assigned identifier of the join line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<SystemTime>,
    /// Time at which the line was received.
    pub received_at: SystemTime,
    /// The joining user's real name, when supplied by the server.
    pub realname: Option<String>,
    /// The joined channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelName,
    /// The joining user.
    pub actor: UserSource,
    /// Whether the joining user is this client's user.
    pub authorship: Authorship,
    /// The netjoin batch that carried this join, when applicable.
    pub netjoin: Option<super::SplitServers>,
}

/// Initial channel membership synchronization completed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ChannelSynced {
    /// The synchronized channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelName,
}

/// A channel's member facts were refreshed by a `WHO` sweep.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MembersRefreshed {
    /// The refreshed channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelName,
}

/// A user left a shared channel with `PART`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Parted {
    /// The server-assigned identifier of the part line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<SystemTime>,
    /// Time at which the line was received.
    pub received_at: SystemTime,
    /// The departing user.
    pub actor: UserSource,
    /// The departed channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelName,
    /// The optional part reason.
    pub reason: Option<String>,
    /// Whether the departing user is this client's user.
    pub authorship: Authorship,
}

/// A user was removed from a shared channel with `KICK`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Kicked {
    /// The server-assigned identifier of the kick line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<SystemTime>,
    /// Time at which the line was received.
    pub received_at: SystemTime,
    /// The actor's account state reported by `account-tag`.
    pub account: Account,
    /// The affected channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelName,
    /// The removed user.
    pub subject: Nick,
    /// The user or server that issued the kick.
    pub actor: Source,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// Whether the removed user is this client's user.
    pub subject_authorship: Authorship,
    /// The optional kick reason.
    pub reason: Option<String>,
}

/// A user was invited to a channel.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Invited {
    /// The invited channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelName,
    /// The invited user.
    pub subject: Nick,
    /// The user or server that sent the invitation.
    pub actor: Source,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// Whether the invited user is this client's user.
    pub subject_authorship: Authorship,
}

/// A channel topic changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TopicChanged {
    /// The server-assigned identifier of the topic line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<SystemTime>,
    /// Time at which the line was received.
    pub received_at: SystemTime,
    /// The actor's account state reported by `account-tag`.
    pub account: Account,
    /// The affected channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelName,
    /// The new topic, or `None` when it was cleared.
    pub topic: Option<String>,
    /// The user or server that changed the topic.
    pub actor: Source,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
}

/// A user or channel mode changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ModeChanged {
    /// The server-assigned identifier of the mode line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<SystemTime>,
    /// Time at which the line was received.
    pub received_at: SystemTime,
    /// The actor's account state reported by `account-tag`.
    pub account: Account,
    /// The user or channel whose modes changed.
    pub subject: Subject,
    /// The user or server that changed the modes.
    pub actor: Source,
    /// Whether the actor is this client's user.
    pub authorship: Authorship,
    /// Whether the distinct subject is this client's user.
    pub subject_authorship: Authorship,
    /// Parsed mode operations in wire order.
    pub changes: Vec<crate::ModeChange>,
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
