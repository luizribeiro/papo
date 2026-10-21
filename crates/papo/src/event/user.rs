use std::time::SystemTime;

use crate::{AccountName, Authorship, Away, ChannelName, MessageId, Nick};

use super::Joined;

/// A user disconnected from the network.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Quit {
    /// The server-assigned identifier of the quit line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, or the local receipt time when none was supplied.
    pub time: SystemTime,
    /// The actor's authenticated account, when tagged.
    pub account: Option<AccountName>,
    /// The disconnected user's nickname.
    pub nick: Nick,
    /// The optional quit reason.
    pub reason: Option<String>,
    /// Shared channels captured before the user was removed.
    pub channels: Vec<ChannelName>,
    /// Whether the disconnected user is this client's user.
    pub authorship: Authorship,
}

/// A user changed nickname.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NickChanged {
    /// The server-assigned identifier of the nick line, when supplied.
    pub msgid: Option<MessageId>,
    /// Valid server time, or the local receipt time when none was supplied.
    pub time: SystemTime,
    /// The actor's authenticated account, when tagged.
    pub account: Option<AccountName>,
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
    /// The affected user's nickname.
    pub nick: Nick,
    /// The user's new presence state.
    pub away: Away,
}

/// A user's account state changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AccountChanged {
    /// The affected user's nickname.
    pub nick: Nick,
    /// The authenticated account, or `None` when logged out.
    pub account: Option<AccountName>,
}

/// A user's username or hostname changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct HostChanged {
    /// The affected user's nickname.
    pub nick: Nick,
    /// The new username from `nick!user@host`.
    pub username: String,
    /// The user's new hostname.
    pub host: String,
}

/// A user's real name changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RealnameChanged {
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
    /// The username from `nick!user@host`, when supplied by the server.
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

/// A user departed during a netsplit.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Departed {
    /// The departed user's nickname.
    pub nick: Nick,
    /// Whether the departed user is this client's user.
    pub authorship: Authorship,
    /// The optional quit reason.
    pub reason: Option<String>,
}

/// Server names attached to a netsplit or netjoin batch, in wire order.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SplitServers {
    /// The first server name in the batch parameters.
    pub first: String,
    /// The second server name in the batch parameters.
    ///
    /// Wire order does not establish which server is nearer to this client.
    pub second: String,
}

impl From<[String; 2]> for SplitServers {
    fn from([first, second]: [String; 2]) -> Self {
        Self { first, second }
    }
}

/// Users departed because connectivity between two servers was lost.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Netsplit {
    /// The server names reported for the lost link.
    pub servers: SplitServers,
    /// The grouped departures in wire order.
    pub quits: Vec<Departed>,
}

/// Users returned after connectivity between two servers was restored.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Netjoin {
    /// The server names reported for the restored link.
    pub servers: SplitServers,
    /// The grouped joins in wire order.
    pub joins: Vec<Joined>,
}
