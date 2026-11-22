use std::time::SystemTime;

use crate::ServerTime;

use crate::{ChannelHandle, ChannelName, MessageId, Nick, Party, Subject};

use super::Actor;

/// A user joined a channel shared with the client.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Joined {
    /// The server-assigned identifier of the join line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<ServerTime>,
    /// Time at which the line was received, from this device's clock.
    pub received_at: SystemTime,
    /// The joined channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelHandle,
    /// The joining user.
    pub actor: crate::UserRef,
    /// Who the joining user is relative to you.
    pub actor_party: Party,
    pub(crate) actor_account: crate::AccountStatus,
    /// The netjoin batch that carried this join, when applicable.
    pub netjoin: Option<super::SplitServers>,
}

/// Initial channel membership synchronization completed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ChannelSynced {
    /// The synchronized channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelHandle,
}

/// A channel's member facts were refreshed by a `WHO` sweep.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MembersRefreshed {
    /// The refreshed channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelHandle,
}

/// A user left a shared channel with `PART`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Parted {
    /// The server-assigned identifier of the part line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<ServerTime>,
    /// Time at which the line was received, from this device's clock.
    pub received_at: SystemTime,
    /// The departing user.
    pub actor: crate::UserRef,
    /// User facts removed by this event, or `None` when the paired state still knows the user.
    pub facts: Option<crate::User>,
    /// The departed channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelHandle,
    /// The optional part reason.
    pub reason: Option<String>,
    /// Who the departing user is relative to you.
    pub actor_party: Party,
    pub(crate) actor_account: crate::AccountStatus,
}

/// A user was removed from a shared channel with `KICK`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Kicked {
    /// The server-assigned identifier of the kick line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<ServerTime>,
    /// Time at which the line was received, from this device's clock.
    pub received_at: SystemTime,
    /// The affected channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelHandle,
    /// The removed user.
    pub subject: crate::UserRef,
    /// User facts removed by this event, or `None` when the paired state still knows the user.
    pub facts: Option<crate::User>,
    /// The user or server that issued the kick.
    pub actor: Actor,
    /// Who the actor is relative to you.
    pub actor_party: Party,
    pub(crate) actor_account: crate::AccountStatus,
    /// Who the removed user is relative to you.
    pub subject_party: Party,
    /// The optional kick reason.
    pub reason: Option<String>,
}

/// A user was invited to a channel.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Invited {
    /// The server-assigned identifier of the invitation line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<ServerTime>,
    /// Time at which the line was received, from this device's clock.
    pub received_at: SystemTime,
    /// The invited channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelHandle,
    /// The invited user.
    pub subject: crate::UserRef,
    /// The user or server that sent the invitation.
    pub actor: Actor,
    /// Who the actor is relative to you.
    pub actor_party: Party,
    pub(crate) actor_account: crate::AccountStatus,
    /// Who the invited user is relative to you.
    pub subject_party: Party,
    pub(crate) receipt: crate::message_ref::Receipt,
}

/// A channel topic changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TopicChanged {
    /// The server-assigned identifier of the topic line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<ServerTime>,
    /// Time at which the line was received, from this device's clock.
    pub received_at: SystemTime,
    /// The affected channel. See [`crate::Conversation`] for status-prefixed line guidance.
    pub channel: ChannelHandle,
    /// The new topic, or `None` when it was cleared.
    pub topic: Option<String>,
    /// The user or server that changed the topic.
    pub actor: Actor,
    /// Who the actor is relative to you.
    pub actor_party: Party,
    pub(crate) actor_account: crate::AccountStatus,
}

super::actor_account_accessor!(TopicChanged);

/// A user or channel mode changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ModeChanged {
    /// The server-assigned identifier of the mode line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<ServerTime>,
    /// Time at which the line was received, from this device's clock.
    pub received_at: SystemTime,
    /// The user or channel whose modes changed.
    pub subject: Subject,
    /// The user or server that changed the modes.
    pub actor: Actor,
    /// Who the actor is relative to you.
    pub actor_party: Party,
    pub(crate) actor_account: crate::AccountStatus,
    /// Parsed mode operations in wire order.
    pub changes: Vec<crate::ModeChange>,
    pub(crate) own_nick: Option<Nick>,
}

super::actor_account_accessor!(ModeChanged);

impl ModeChanged {
    /// Returns who the user a mode change is about is relative to you.
    #[must_use]
    pub fn subject_party(&self, change: &crate::ModeChange) -> Party {
        match (&change.arg, &self.subject) {
            (Some(crate::ModeArg::Member { user, .. }), _) => self.nick_party(user.reported_nick()),
            (_, Subject::User(user)) => self.nick_party(&user.nick()),
            (_, Subject::Channel(_)) => Party::Other,
        }
    }

    fn nick_party(&self, affected: &Nick) -> Party {
        match self.own_nick.as_ref() {
            Some(own) if own == affected => Party::Me,
            Some(_) => Party::Other,
            None => Party::Unknown,
        }
    }
}

/// A joined channel was renamed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ChannelRenamed {
    /// The server-assigned identifier of the rename line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<ServerTime>,
    /// Time at which the line was received, from this device's clock.
    pub received_at: SystemTime,
    /// The renamed channel lifecycle. See [`crate::Conversation`] for status-prefixed line
    /// guidance.
    pub channel: ChannelHandle,
    /// The previous channel name.
    pub old: ChannelName,
    /// The new channel name.
    pub new: ChannelName,
    /// The user or server that renamed the channel.
    pub actor: Actor,
    /// Who the actor is relative to you.
    pub actor_party: Party,
    pub(crate) actor_account: crate::AccountStatus,
    /// The server-supplied reason, when present and non-empty.
    pub reason: Option<String>,
}

super::actor_account_accessor!(ChannelRenamed);

super::actor_account_accessor!(Joined, Parted, Kicked, Invited);
