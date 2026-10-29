use std::time::SystemTime;

use crate::{Account, Authorship, Away, ChannelName, MessageId, Nick, ServerName};

use super::UserSource;

/// A user disconnected from the network.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Quit {
    /// The server-assigned identifier of the quit line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<SystemTime>,
    /// Time at which the line was received.
    pub received_at: SystemTime,
    /// The disconnected user.
    pub actor: UserSource,
    /// The optional quit reason.
    pub reason: Option<String>,
    /// Shared channels where papo knows the user is a member.
    pub channels: Vec<ChannelName>,
    /// Whether the disconnected user is this client's user.
    pub authorship: Authorship,
    /// The netsplit batch that carried this quit, when applicable.
    pub netsplit: Option<SplitServers>,
}

/// A user changed nickname.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NickChanged {
    /// The server-assigned identifier of the nick line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, when supplied.
    pub server_time: Option<SystemTime>,
    /// Time at which the line was received.
    pub received_at: SystemTime,
    /// The renamed user before the change.
    pub actor: UserSource,
    /// The new nickname.
    pub new: Nick,
    /// Shared channels where papo knows the user is a member.
    pub channels: Vec<ChannelName>,
    /// Whether the renamed user is this client's user.
    pub authorship: Authorship,
}

/// A user's away state changed.
///
/// This derived event keeps the triggering line's time but not its message identifier, so it
/// cannot collide with the typed event produced from the same line during deduplication.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AwayChanged {
    /// Valid server time on the line that caused the change, when supplied.
    pub server_time: Option<SystemTime>,
    /// Time at which the triggering line was received.
    pub received_at: SystemTime,
    /// The affected user.
    pub actor: UserSource,
    /// The user's new presence state.
    pub new: Away,
    /// Whether the affected user is this client's user.
    pub authorship: Authorship,
}

/// A user's account state changed.
///
/// This derived event keeps the triggering line's time but not its message identifier, so it
/// cannot collide with the typed event produced from the same line during deduplication.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AccountChanged {
    /// Valid server time on the line that caused the change, when supplied.
    pub server_time: Option<SystemTime>,
    /// Time at which the triggering line was received.
    pub received_at: SystemTime,
    /// The affected user.
    pub actor: UserSource,
    /// The account state after the change.
    ///
    /// This is separate from the account fact carried by [`Self::actor`].
    pub new: Account,
    /// Whether the affected user is this client's user.
    pub authorship: Authorship,
}

/// A user's username or hostname changed.
///
/// This derived event keeps the triggering line's time but not its message identifier, so it
/// cannot collide with the typed event produced from the same line during deduplication.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct HostChanged {
    /// Valid server time on the line that caused the change, when supplied.
    pub server_time: Option<SystemTime>,
    /// Time at which the triggering line was received.
    pub received_at: SystemTime,
    /// The affected user.
    pub actor: UserSource,
    /// The new username from `nick!user@host`; it may start with `~`.
    pub new_username: String,
    /// The user's new hostname.
    pub new_host: String,
    /// Whether the affected user is this client's user.
    pub authorship: Authorship,
}

/// A user's real name changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RealnameChanged {
    /// The affected user.
    pub actor: UserSource,
    /// The user's new real name.
    pub new: String,
    /// Whether the affected user is this client's user.
    pub authorship: Authorship,
}

/// A monitored nickname came online.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MonitorOnline {
    /// The monitored user reported online.
    pub subject: UserSource,
    pub(crate) receipt: crate::message_ref::Receipt,
}

/// A monitored nickname went offline.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MonitorOffline {
    /// The monitored nickname.
    pub subject: Nick,
}

/// A netsplit or netjoin batch and its server names, in wire order.
///
/// Consecutive events with equal markers belong to the same batch:
/// ```
/// # use papo::Event;
/// # fn group(events: &[Event]) {
/// let mut previous = None;
/// for event in events {
///     if let Event::Quit(quit) = event
///         && quit.netsplit.as_ref() != previous
///     {
///         previous = quit.netsplit.as_ref();
///         // Start a new displayed group when `previous` is `Some`.
///     }
/// }
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SplitServers {
    /// The server's opaque batch reference.
    pub batch: String,
    /// The first server name in the batch parameters.
    pub first: ServerName,
    /// The second server name in the batch parameters.
    ///
    /// Wire order does not establish which server is nearer to this client.
    pub second: ServerName,
}

impl SplitServers {
    pub(super) fn from_core(value: papo_core::SplitServers) -> Self {
        Self {
            batch: value.batch,
            first: value.first.into(),
            second: value.second.into(),
        }
    }
}
