use std::time::SystemTime;

use crate::{Account, AccountName, Authorship, Away, ChannelName, MessageId, Nick};

use super::Source;

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
    /// The actor's account state reported by `account-tag`.
    pub account: Account,
    /// The disconnected user.
    pub actor: Source,
    /// The disconnected user's nickname.
    pub nick: Nick,
    /// The optional quit reason.
    pub reason: Option<String>,
    /// Shared channels captured before the user was removed.
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
    /// The actor's account state reported by `account-tag`.
    pub account: Account,
    /// The renamed user before the change.
    pub actor: Source,
    /// The previous nickname.
    pub old: Nick,
    /// The new nickname.
    pub new: Nick,
    /// Whether the renamed user is this client's user.
    pub authorship: Authorship,
}

/// A user's away state changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AwayChanged {
    /// The affected user.
    pub actor: Source,
    /// The affected user's nickname.
    pub nick: Nick,
    /// The user's new presence state.
    pub away: Away,
}

/// A user's account state changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AccountChanged {
    /// The affected user.
    pub actor: Source,
    /// The affected user's nickname.
    pub nick: Nick,
    /// The authenticated account, or `None` when logged out.
    pub account: Option<AccountName>,
}

/// A user's username or hostname changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct HostChanged {
    /// The affected user.
    pub actor: Source,
    /// The affected user's nickname.
    pub nick: Nick,
    /// The new username from `nick!user@host`; it may start with `~`.
    pub username: String,
    /// The user's new hostname.
    pub host: String,
}

/// A user's real name changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RealnameChanged {
    /// The affected user.
    pub actor: Source,
    /// The affected user's nickname.
    pub nick: Nick,
    /// The user's new real name.
    pub realname: String,
}

/// A monitored nickname came online.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MonitorOnline {
    /// The monitored nickname.
    pub nick: Nick,
    /// The username from `nick!user@host`, when supplied by the server; it may start with `~`.
    pub username: Option<String>,
    /// The user's current hostname, when supplied by the server.
    pub host: Option<String>,
}

/// A monitored nickname went offline.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MonitorOffline {
    /// The monitored nickname.
    pub nick: Nick,
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
    pub first: String,
    /// The second server name in the batch parameters.
    ///
    /// Wire order does not establish which server is nearer to this client.
    pub second: String,
}

impl SplitServers {
    pub(super) fn from_core(value: papo_core::SplitServers) -> Self {
        Self {
            batch: value.batch,
            first: value.first,
            second: value.second,
        }
    }
}
