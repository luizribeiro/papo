use crate::{AccountName, ChannelName, Nick};

use super::Joined;

/// A user disconnected from the network.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Quit {
    /// The disconnected user's nickname.
    pub nick: Nick,
    /// The optional quit reason.
    pub reason: Option<String>,
    /// Shared channels captured before the user was removed.
    pub channels: Vec<ChannelName>,
}

/// A user changed nickname.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NickChanged {
    /// The previous nickname.
    pub old: Nick,
    /// The new nickname.
    pub new: Nick,
    /// Whether the renamed user is this client.
    pub is_self: bool,
}

/// A user's away state changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AwayChanged {
    /// The affected user's nickname.
    pub nick: Nick,
    /// The away message, or `None` when the user returned.
    pub message: Option<String>,
}

/// A user's account state changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AccountChanged {
    /// The affected user's nickname.
    pub nick: Nick,
    /// The authenticated account, or `None` when logged out or unknown.
    pub account: Option<AccountName>,
}

/// A user's username or hostname changed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct HostChanged {
    /// The affected user's nickname.
    pub nick: Nick,
    /// The user's new username.
    pub user: String,
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

/// A monitored nickname came online or changed its userhost.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MonitorOnline {
    /// The monitored nickname.
    pub nick: Nick,
    /// The user's current username, when supplied by the server.
    pub user: Option<String>,
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
    /// The optional quit reason.
    pub reason: Option<String>,
}

/// Users departed because connectivity between two servers was lost.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Netsplit {
    /// The servers on either side of the lost link.
    pub servers: [String; 2],
    /// The grouped departures in wire order.
    pub quits: Vec<Departed>,
}

/// Users returned after connectivity between two servers was restored.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Netjoin {
    /// The servers on either side of the restored link.
    pub servers: [String; 2],
    /// The grouped joins in wire order.
    pub joins: Vec<Joined>,
}
