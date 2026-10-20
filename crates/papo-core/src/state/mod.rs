#[cfg(feature = "unstable")]
use std::collections::BTreeMap;

pub(crate) mod membership;
pub(crate) mod metadata;
#[cfg(feature = "unstable")]
pub(crate) mod persistence;
pub(crate) mod presence;
pub(crate) mod read_marker;
pub(crate) mod rename;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use papo_proto::{
    Account as AccountNotify, AccountName, Away as AwayNotify, Casemapping, ChannelName, Chghost,
    Command, Folded, Isupport, Join, LineRef, ModeChanges, ModeKind, NamesEntry, Nick, Numeric,
    Setname, Source, Target, WhoxFlags, WhoxPresence, WhoxReply, parse_353, time,
};

#[cfg(feature = "unstable")]
use crate::{
    BouncerNetwork, NetworkId, PersistenceSetting, PersistenceStatus, bouncer::NetworkUpdate,
};
use crate::{
    Config, Event, Handled, LastSeen, MetadataValue, Outbox, OwnedModeChange, ReadMarker,
    StateLimit,
    monitor::{Monitor, presence_nicks},
};

const WHOX_FIELDS: &str = "%tcuhnfar,152";
const WHOX_TOKEN: &str = "152";
const MAX_LAST_SEEN_TARGETS: usize = 1_000;
const MAX_METADATA_TARGETS: usize = 10_000;
pub(crate) const MAX_METADATA_KEYS_PER_TARGET: usize = 10_000;
#[cfg(feature = "unstable")]
const MAX_BOUNCER_NETWORKS: usize = 10_000;
/// The known channel and user state for a session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct State {
    nick: Nick,
    user: String,
    host: Option<String>,
    realname: String,
    away: bool,
    away_message: Option<String>,
    user_modes: HashSet<char>,
    casemapping: Casemapping,
    channels: HashMap<Folded<ChannelName>, Channel>,
    users: HashMap<Folded<Nick>, User>,
    who_sweeps: HashSet<Folded<ChannelName>>,
    read_markers: HashMap<Folded<Target>, ReadMarker>,
    last_seen: HashMap<Folded<Target>, LastSeen>,
    last_seen_order: VecDeque<String>,
    max_channels: usize,
    max_members_per_channel: usize,
    channel_limit_warned: bool,
    member_limit_warned: bool,
    #[cfg(feature = "unstable")]
    persistence: Option<PersistenceStatus>,
    #[cfg(feature = "unstable")]
    bouncer_networks: BTreeMap<NetworkId, BouncerNetwork>,
    metadata: HashMap<Folded<Target>, HashMap<String, MetadataValue>>,
    metadata_subscriptions: Vec<String>,
    limits_warned: HashSet<StateLimit>,
}

/// State known about a joined channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Channel {
    name: ChannelName,
    casemapping: Casemapping,
    members: HashMap<Folded<Nick>, Member>,
    modes: HashMap<char, Option<String>>,
    created_at: Option<SystemTime>,
    topic: Option<String>,
    topic_set_by: Option<String>,
    topic_set_at: Option<SystemTime>,
    names_reply_seen: bool,
    synced: bool,
    prefix_order: Arc<[char]>,
}

/// State known about a channel member.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Member {
    nick: Nick,
    prefixes: Vec<char>,
    prefix_order: Arc<[char]>,
}

/// State known about a user sharing a channel or followed through MONITOR.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct User {
    nick: Nick,
    username: Option<String>,
    host: Option<String>,
    account: Account,
    away: Away,
    realname: Option<String>,
    is_bot: bool,
    monitored: bool,
    channels: HashSet<Folded<ChannelName>>,
}

/// A user's known account state.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Account {
    /// No source has reported whether the user is logged in.
    Unknown,
    /// The user is known to be logged out.
    LoggedOut,
    /// The user is logged in to the named account.
    LoggedIn(AccountName),
}

/// A user's known presence state.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Away {
    /// The user is known to be present.
    Present,
    /// The user is away, with a message when the server supplied one.
    Away {
        /// The away message, or `None` when a WHO reply reported only the away flag.
        message: Option<String>,
    },
}

#[derive(Clone, Copy)]
struct WhoUser<'a> {
    channel: &'a str,
    nick: &'a str,
    username: Option<&'a str>,
    host: Option<&'a str>,
    realname: Option<&'a str>,
}

#[derive(Clone, Copy)]
pub(crate) struct MetadataContext {
    pub(crate) max_keys: usize,
    pub(crate) subscription_batch: bool,
}

#[derive(Eq, PartialEq)]
pub(crate) enum StateSnapshot {
    Full(Box<State>),
    Metadata {
        target: Folded<Target>,
        key: String,
        value: Option<MetadataValue>,
    },
    Monitor(Vec<(Folded<Nick>, Option<User>)>),
    #[cfg(feature = "unstable")]
    Bouncer {
        id: NetworkId,
        network: Option<BouncerNetwork>,
    },
}

#[cfg(test)]
impl StateSnapshot {
    fn comparison_steps(&self) -> Option<usize> {
        match self {
            Self::Full(_) => None,
            Self::Metadata { .. } => Some(1),
            Self::Monitor(entries) => Some(entries.len()),
            #[cfg(feature = "unstable")]
            Self::Bouncer { .. } => Some(1),
        }
    }
}

fn metadata_update<'a>(
    message: &LineRef<'a>,
) -> Option<(&'a str, &'a str, &'a str, Option<&'a str>)> {
    match message.command() {
        Command::Numeric(Numeric::RPL_KEYVALUE) => Some((
            message.param(1)?,
            message.param(2)?,
            message.param(3)?,
            Some(message.param(4)?),
        )),
        Command::Numeric(Numeric::RPL_KEYNOTSET) => {
            Some((message.param(1)?, message.param(2)?, "*", None))
        }
        Command::Metadata => Some((
            message.param(0)?,
            message.param(1)?,
            message.param(2)?,
            message.param(3),
        )),
        _ => None,
    }
}

impl State {
    pub(crate) fn snapshot_for(&self, message: &LineRef<'_>) -> StateSnapshot {
        if let Some((target, key, _, _)) = metadata_update(message) {
            let target = Folded::new(target, self.casemapping);
            let value = self
                .metadata
                .get(&target)
                .and_then(|values| values.get(key))
                .cloned();
            return StateSnapshot::Metadata {
                target,
                key: key.to_owned(),
                value,
            };
        }
        let nicks = presence_nicks(message);
        if !nicks.is_empty() {
            return StateSnapshot::Monitor(
                nicks
                    .into_iter()
                    .map(|nick| {
                        let key = Folded::new(nick, self.casemapping);
                        let user = self.users.get(&key).cloned();
                        (key, user)
                    })
                    .collect(),
            );
        }
        #[cfg(feature = "unstable")]
        if let Some(update) = NetworkUpdate::parse(message) {
            let network = self.bouncer_networks.get(&update.id).cloned();
            return StateSnapshot::Bouncer {
                id: update.id,
                network,
            };
        }
        StateSnapshot::Full(Box::new(self.clone()))
    }

    pub(crate) fn new(config: &Config) -> Self {
        Self {
            nick: Casemapping::default().nick(&config.nick),
            user: config.username.clone(),
            host: None,
            realname: config.realname.clone(),
            away: false,
            away_message: None,
            user_modes: HashSet::new(),
            casemapping: Casemapping::default(),
            channels: HashMap::new(),
            users: HashMap::new(),
            who_sweeps: HashSet::new(),
            read_markers: HashMap::new(),
            last_seen: HashMap::new(),
            last_seen_order: VecDeque::new(),
            max_channels: config.max_channels,
            max_members_per_channel: config.max_members_per_channel,
            channel_limit_warned: false,
            member_limit_warned: false,
            #[cfg(feature = "unstable")]
            persistence: None,
            #[cfg(feature = "unstable")]
            bouncer_networks: BTreeMap::new(),
            metadata: HashMap::new(),
            metadata_subscriptions: Vec::new(),
            limits_warned: HashSet::new(),
        }
    }

    pub(crate) fn reset(&mut self, config: &Config) {
        let last_seen = std::mem::take(&mut self.last_seen);
        let last_seen_order = std::mem::take(&mut self.last_seen_order);
        *self = Self::new(config);
        self.last_seen = last_seen
            .into_values()
            .map(|seen| (Folded::new(&seen.target, self.casemapping), seen))
            .collect();
        self.last_seen_order = last_seen_order;
    }

    pub(crate) fn sync_monitored(&mut self, monitor: &Monitor, isupport: &Isupport) {
        self.update_casemapping(isupport.casemapping());
        for user in self.users.values_mut() {
            user.monitored = false;
        }
        for (nick, username, host) in monitor.online() {
            let user = self
                .users
                .entry(Folded::new(nick, self.casemapping))
                .or_insert_with(|| User::new(nick, username, host, self.casemapping));
            user.monitored = true;
        }
        self.users
            .retain(|_, user| user.monitored || !user.channels.is_empty());
    }

    pub(crate) fn sync_monitored_nick(
        &mut self,
        monitor: &Monitor,
        isupport: &Isupport,
        nick: &str,
    ) {
        self.update_casemapping(isupport.casemapping());
        let key = Folded::new(nick, self.casemapping);
        if let Some((username, host)) = monitor.online_nick(nick, isupport) {
            self.users
                .entry(key)
                .or_insert_with(|| User::new(nick, username, host, self.casemapping))
                .monitored = true;
        } else if let Some(user) = self.users.get_mut(&key) {
            user.monitored = false;
            if user.channels.is_empty() {
                self.users.remove(&key);
            }
        }
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(crate) fn handle_message(
        &mut self,
        message: &LineRef<'_>,
        own_nick: &str,
        isupport: &Isupport,
        no_implicit_names: bool,
        metadata: MetadataContext,
        own_away_message: Option<&str>,
        outbox: &mut Outbox,
    ) -> Handled {
        let mapping = self.prepare_message(message, own_nick, isupport, outbox);
        if self.handle_metadata(
            message,
            metadata.max_keys,
            metadata.subscription_batch,
            outbox,
        ) {
            return Handled::Yes;
        }
        #[cfg(feature = "unstable")]
        if self.handle_persistence(message, outbox) {
            return Handled::Yes;
        }
        #[cfg(feature = "unstable")]
        if self.handle_bouncer(message, outbox) {
            return Handled::Yes;
        }
        self.remember_last_seen(message, own_nick);
        if self.handle_read_marker(message, outbox) {
            return Handled::Yes;
        }
        if let Some(join) = Join::parse(message) {
            return self.handle_join(message, join, isupport, no_implicit_names, outbox);
        }
        if let Some(handled) = self.handle_user_update(message, isupport, own_away_message, outbox)
        {
            return handled;
        }
        if message.command() == Command::Part {
            let Some(Source::User { nick, .. }) = message.source() else {
                return Handled::No;
            };
            let Some(channels) = message.param(0) else {
                return Handled::No;
            };
            for channel in channels.split(',') {
                outbox.event(Event::Parted {
                    channel: channel.to_owned(),
                    nick: nick.to_owned(),
                    reason: message.param(1).map(str::to_owned),
                });
                if mapping.eq(nick, &self.nick) {
                    self.remove_channel(channel);
                } else {
                    self.remove_member(channel, nick);
                }
            }
            return Handled::Yes;
        }
        if message.command() == Command::Kick {
            let (Some(source), Some(channel), Some(nick)) =
                (message.source(), message.param(0), message.param(1))
            else {
                return Handled::No;
            };
            let by = match source {
                Source::Server(name) => name,
                Source::User { nick, .. } => nick,
            };
            outbox.event(Event::Kicked {
                channel: channel.to_owned(),
                nick: nick.to_owned(),
                by: by.to_owned(),
                reason: message.param(2).map(str::to_owned),
            });
            if mapping.eq(nick, &self.nick) {
                self.remove_channel(channel);
            } else {
                self.remove_member(channel, nick);
            }
            return Handled::Yes;
        }
        if message.command() == Command::Quit {
            return self.handle_quit(message, outbox);
        }
        if message.command() == Command::Nick {
            let (Some(Source::User { nick, .. }), Some(new_nick)) =
                (message.source(), message.param(0))
            else {
                return Handled::No;
            };
            self.rename_user(nick, new_nick);
            if mapping.eq(nick, &self.nick) {
                self.nick = mapping.nick(new_nick);
            }
            outbox.event(Event::NickChanged {
                old: nick.to_owned(),
                new: new_nick.to_owned(),
            });
            return Handled::Yes;
        }
        if self.handle_channel_rename(message, outbox) {
            return Handled::Yes;
        }
        let mode_reported = Self::report_mode_change(message, isupport, outbox);
        if self.handle_channel_mode(message, isupport)
            || self.handle_channel_metadata(message, isupport)
            || self.handle_user_modes(message)
            || self.handle_topic(message, outbox)
            || mode_reported
        {
            return Handled::Yes;
        }
        if let Some((_, channel, entries)) = parse_353(message) {
            let Some(state) = self
                .channels
                .get_mut(&Folded::new(channel, self.casemapping))
            else {
                return Handled::No;
            };
            state.names_reply_seen = true;
            for raw in entries {
                let entry = NamesEntry::parse(raw, isupport);
                if !entry.nick.is_empty() {
                    self.add_member(
                        channel,
                        entry.nick,
                        entry.user,
                        entry.host,
                        entry.prefixes,
                        outbox,
                    );
                }
            }
            return Handled::Yes;
        }
        if self.finish_names(message, isupport, no_implicit_names, outbox) {
            return Handled::Yes;
        }
        Handled::No
    }

    pub(crate) fn handle_whoami(
        &mut self,
        message: &LineRef<'_>,
        own_nick: &str,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> bool {
        let Some(change) = Setname::parse(message) else {
            return false;
        };
        let Some(Source::User {
            nick,
            user: Some(user),
            host: Some(host),
        }) = message.source()
        else {
            return false;
        };
        self.update_casemapping(isupport.casemapping());
        if !self.casemapping.eq(nick, own_nick) {
            return false;
        }
        let realname_changed = self.realname != change.realname;
        self.nick = self.casemapping.nick(nick);
        user.clone_into(&mut self.user);
        self.host = Some(host.to_owned());
        change.realname.clone_into(&mut self.realname);
        if realname_changed {
            outbox.event(Event::RealnameChanged {
                nick: nick.to_owned(),
                realname: change.realname.to_owned(),
            });
        }
        true
    }

    #[cfg(feature = "unstable")]
    fn handle_bouncer(&mut self, message: &LineRef<'_>, outbox: &mut Outbox) -> bool {
        let Some(update) = NetworkUpdate::parse(message) else {
            return false;
        };
        if update.removed {
            self.bouncer_networks.remove(&update.id);
            outbox.event(Event::BouncerNetwork {
                id: update.id,
                network: None,
            });
            return true;
        }
        if !self.bouncer_networks.contains_key(&update.id)
            && self.bouncer_networks.len() >= MAX_BOUNCER_NETWORKS
        {
            self.warn_limit(StateLimit::BouncerNetworks, outbox);
            return true;
        }
        let network = self
            .bouncer_networks
            .entry(update.id.clone())
            .or_insert_with(|| BouncerNetwork::empty(update.id.clone()));
        network.apply(update.attributes);
        outbox.event(Event::BouncerNetwork {
            id: update.id,
            network: Some(network.clone()),
        });
        true
    }

    fn handle_channel_mode(&mut self, message: &LineRef<'_>, isupport: &Isupport) -> bool {
        let Some(target) = message
            .param(0)
            .filter(|target| message.command() == Command::Mode && isupport.is_channel(target))
        else {
            return false;
        };
        if let Some(modestring) = message.param(1)
            && let Some(channel) = self
                .channels
                .get_mut(&Folded::new(target, self.casemapping))
        {
            channel.apply_modes(modestring, message.params().skip(2), isupport, false);
        }
        true
    }

    fn report_mode_change(message: &LineRef<'_>, isupport: &Isupport, outbox: &mut Outbox) -> bool {
        let (Some(target), Some(modestring), Some(by)) = (
            message
                .param(0)
                .filter(|_| message.command() == Command::Mode),
            message.param(1),
            source_name(message),
        ) else {
            return false;
        };
        let changes = if isupport.is_channel(target) {
            ModeChanges::parse(modestring, message.params().skip(2), isupport)
                .map(OwnedModeChange::from)
                .collect()
        } else {
            ModeChanges::parse(modestring, message.params().skip(2), &Isupport::default())
                .map(OwnedModeChange::from)
                .collect()
        };
        outbox.event(Event::ModeChanged {
            target: target.to_owned(),
            by: by.to_owned(),
            changes,
        });
        true
    }

    fn handle_channel_metadata(&mut self, message: &LineRef<'_>, isupport: &Isupport) -> bool {
        if !matches!(
            message.command(),
            Command::Numeric(Numeric::RPL_CHANNELMODEIS | Numeric::RPL_CREATIONTIME)
        ) {
            return false;
        }
        let Some(channel) = message
            .param(1)
            .and_then(|name| self.channels.get_mut(&Folded::new(name, self.casemapping)))
        else {
            return false;
        };
        if message.command() == Command::Numeric(Numeric::RPL_CHANNELMODEIS) {
            if let Some(modestring) = message.param(2) {
                channel.apply_modes(modestring, message.params().skip(3), isupport, true);
            }
        } else {
            channel.created_at = message.param(2).and_then(parse_unix_seconds);
        }
        true
    }

    fn handle_user_modes(&mut self, message: &LineRef<'_>) -> bool {
        let modestring = match message.command() {
            Command::Mode
                if message
                    .param(0)
                    .is_some_and(|nick| self.casemapping.eq(nick, &self.nick)) =>
            {
                message.param(1)
            }
            Command::Numeric(Numeric::RPL_UMODEIS) => {
                self.user_modes.clear();
                message.param(1)
            }
            _ => return false,
        };
        let mut set = true;
        for mode in modestring.unwrap_or_default().chars() {
            match mode {
                '+' => set = true,
                '-' => set = false,
                _ if set => _ = self.user_modes.insert(mode),
                _ => _ = self.user_modes.remove(&mode),
            }
        }
        true
    }

    fn handle_topic(&mut self, message: &LineRef<'_>, outbox: &mut Outbox) -> bool {
        let (channel_name, topic, live) = match message.command() {
            Command::Numeric(Numeric::RPL_TOPIC) => (message.param(1), message.param(2), false),
            Command::Topic => (message.param(0), message.param(1), true),
            Command::Numeric(Numeric::RPL_TOPICWHOTIME) => {
                let Some(channel) = message
                    .param(1)
                    .and_then(|name| self.channels.get_mut(&Folded::new(name, self.casemapping)))
                else {
                    return false;
                };
                channel.topic_set_by = message.param(2).map(str::to_owned);
                channel.topic_set_at = message.param(3).and_then(parse_unix_seconds);
                return true;
            }
            _ => return false,
        };
        if live && let (Some(channel), Some(by)) = (channel_name, source_name(message)) {
            outbox.event(Event::TopicChanged {
                channel: channel.to_owned(),
                topic: topic.filter(|topic| !topic.is_empty()).map(str::to_owned),
                by: by.to_owned(),
            });
        }
        let Some(channel) = channel_name
            .and_then(|name| self.channels.get_mut(&Folded::new(name, self.casemapping)))
        else {
            return false;
        };
        channel.topic = topic.filter(|topic| !topic.is_empty()).map(str::to_owned);
        channel.topic_set_by = if live {
            message.source().map(|source| match source {
                Source::Server(name) => name.to_owned(),
                Source::User { nick, .. } => nick.to_owned(),
            })
        } else {
            None
        };
        channel.topic_set_at = None;
        true
    }

    fn update_casemapping(&mut self, mapping: Casemapping) {
        if mapping == self.casemapping {
            return;
        }
        self.channels = std::mem::take(&mut self.channels)
            .into_values()
            .map(|mut channel| {
                channel.casemapping = mapping;
                channel.members = channel
                    .members
                    .into_values()
                    .map(|member| (Folded::new(member.nick.as_ref(), mapping), member))
                    .collect();
                (Folded::new(channel.name.as_ref(), mapping), channel)
            })
            .collect();
        self.users = std::mem::take(&mut self.users)
            .into_values()
            .map(|mut user| {
                user.channels = user
                    .channels
                    .into_iter()
                    .map(|channel| Folded::new(channel.original(), mapping))
                    .collect();
                (Folded::new(user.nick.as_ref(), mapping), user)
            })
            .collect();
        self.who_sweeps = std::mem::take(&mut self.who_sweeps)
            .into_iter()
            .map(|channel| Folded::new(channel.original(), mapping))
            .collect();
        self.read_markers = std::mem::take(&mut self.read_markers)
            .into_values()
            .map(|marker| (Folded::new(&marker.target, mapping), marker))
            .collect();
        self.last_seen = std::mem::take(&mut self.last_seen)
            .into_values()
            .map(|seen| (Folded::new(&seen.target, mapping), seen))
            .collect();
        self.metadata = std::mem::take(&mut self.metadata)
            .into_iter()
            .map(|(target, values)| (Folded::new(target.original(), mapping), values))
            .collect();
        self.casemapping = mapping;
    }

    /// Returns the session's current nickname.
    #[must_use]
    pub fn nick(&self) -> &Nick {
        &self.nick
    }

    /// Returns the session's username.
    #[must_use]
    pub fn username(&self) -> &str {
        &self.user
    }

    /// Returns the session's hostname when known.
    #[must_use]
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// Returns the session's real name.
    #[must_use]
    pub fn realname(&self) -> &str {
        &self.realname
    }

    /// Returns whether the session is marked away.
    #[must_use]
    pub const fn away(&self) -> bool {
        self.away
    }

    /// Returns this session's confirmed away message.
    ///
    /// `None` means the session is not away or the server confirmed away state without echoing a
    /// message that papo sent through the typed away action.
    #[must_use]
    pub fn away_message(&self) -> Option<&str> {
        self.away_message.as_deref()
    }

    /// Returns a joined channel using the server's current casemapping.
    #[must_use]
    pub fn channel(&self, name: impl AsRef<str>) -> Option<&Channel> {
        let name = self.casemapping.fold(name.as_ref());
        self.channels.get(name.as_ref())
    }

    /// Iterates over joined channels in unspecified order.
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.channels.values()
    }

    /// Iterates over upstream networks reported by the bouncer.
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub fn bouncer_networks(&self) -> impl Iterator<Item = &BouncerNetwork> {
        self.bouncer_networks.values()
    }

    /// Iterates over the session's current user mode letters in unspecified order.
    pub fn user_modes(&self) -> impl Iterator<Item = char> + '_ {
        self.user_modes.iter().copied()
    }

    /// Returns a shared user using the server's current casemapping.
    #[must_use]
    pub fn user(&self, nick: impl AsRef<str>) -> Option<&User> {
        let nick = self.casemapping.fold(nick.as_ref());
        self.users.get(nick.as_ref())
    }

    /// Returns cached metadata using the server's current casemapping.
    #[must_use]
    pub fn metadata(&self, target: impl AsRef<str>) -> Option<&HashMap<String, MetadataValue>> {
        let target = self.casemapping.fold(target.as_ref());
        self.metadata.get(target.as_ref())
    }

    /// Returns the number of targets retained in the metadata cache.
    #[must_use]
    pub fn metadata_target_count(&self) -> usize {
        self.metadata.len()
    }

    /// Returns metadata keys confirmed in the server-side subscription list.
    #[must_use]
    pub fn metadata_subscriptions(&self) -> &[String] {
        &self.metadata_subscriptions
    }

    pub(crate) fn clear_metadata_subscriptions(&mut self) {
        self.metadata_subscriptions.clear();
    }

    /// Returns the newest read marker reported for a target.
    #[must_use]
    pub fn read_marker(&self, target: impl AsRef<str>) -> Option<&ReadMarker> {
        let target = self.casemapping.fold(target.as_ref());
        self.read_markers.get(target.as_ref())
    }

    /// Returns the newest live message remembered for a target.
    #[must_use]
    pub fn last_seen(&self, target: impl AsRef<str>) -> Option<&LastSeen> {
        let target = self.casemapping.fold(target.as_ref());
        self.last_seen.get(target.as_ref())
    }

    /// Iterates over users currently reported online through `MONITOR`.
    ///
    /// Presence from `730` replies is included without requiring `extended-monitor`.
    pub fn monitor_online(&self) -> impl Iterator<Item = &User> {
        self.users.values().filter(|user| user.monitored)
    }
}

fn source_name<'a>(message: &'a LineRef<'_>) -> Option<&'a str> {
    Some(match message.source()? {
        Source::Server(name) => name,
        Source::User { nick, .. } => nick,
    })
}

impl Channel {
    fn new(name: &str, casemapping: Casemapping, isupport: &Isupport, synced: bool) -> Self {
        Self {
            name: casemapping.channel(name),
            casemapping,
            members: HashMap::new(),
            modes: HashMap::new(),
            created_at: None,
            topic: None,
            topic_set_by: None,
            topic_set_at: None,
            names_reply_seen: false,
            synced,
            prefix_order: isupport
                .prefix()
                .into_iter()
                .map(|prefix| prefix.symbol)
                .collect(),
        }
    }

    fn apply_modes<'a>(
        &mut self,
        modestring: &'a str,
        args: impl Iterator<Item = &'a str> + 'a,
        isupport: &'a Isupport,
        replace: bool,
    ) {
        if replace {
            self.modes.clear();
        }
        let prefixes = isupport.prefix();
        for change in ModeChanges::parse(modestring, args, isupport) {
            match change.kind {
                ModeKind::Prefix(rank) => {
                    let (Some(nick), Some(prefix)) = (change.arg, prefixes.get(rank)) else {
                        continue;
                    };
                    let Some(member) = self.members.get_mut(&Folded::new(nick, self.casemapping))
                    else {
                        continue;
                    };
                    if change.set && !member.prefixes.contains(&prefix.symbol) {
                        member.prefixes.push(prefix.symbol);
                        member.prefixes.sort_by_key(|symbol| {
                            member.prefix_order.iter().position(|item| item == symbol)
                        });
                    } else if !change.set {
                        member.prefixes.retain(|symbol| *symbol != prefix.symbol);
                    }
                }
                ModeKind::AlwaysArg | ModeKind::ArgWhenSet | ModeKind::NoArg => {
                    if change.set && (change.arg.is_some() || change.kind == ModeKind::NoArg) {
                        if self.modes.len() < 64 || self.modes.contains_key(&change.mode) {
                            self.modes
                                .insert(change.mode, change.arg.map(str::to_owned));
                        }
                    } else if !change.set {
                        self.modes.remove(&change.mode);
                    }
                }
                ModeKind::List | ModeKind::Unknown => {}
            }
        }
    }

    /// Returns the channel name as received from the server.
    #[must_use]
    pub fn name(&self) -> &ChannelName {
        &self.name
    }

    /// Returns the current topic when known.
    #[must_use]
    pub fn topic(&self) -> Option<&str> {
        self.topic.as_deref()
    }

    /// Returns who set the current topic when known.
    #[must_use]
    pub fn topic_set_by(&self) -> Option<&str> {
        self.topic_set_by.as_deref()
    }

    /// Returns when the current topic was set.
    #[must_use]
    pub const fn topic_set_at(&self) -> Option<SystemTime> {
        self.topic_set_at
    }

    /// Returns a stored non-list mode's argument, distinguishing flags from absent modes.
    #[must_use]
    pub fn mode(&self, mode: char) -> Option<Option<&str>> {
        self.modes.get(&mode).map(|argument| argument.as_deref())
    }

    /// Iterates over stored non-list modes in unspecified order.
    pub fn modes(&self) -> impl Iterator<Item = (char, Option<&str>)> {
        self.modes
            .iter()
            .map(|(mode, argument)| (*mode, argument.as_deref()))
    }

    /// Returns the channel creation time.
    #[must_use]
    pub const fn created_at(&self) -> Option<SystemTime> {
        self.created_at
    }

    /// Returns whether initial member synchronization is complete.
    ///
    /// Papo requests a names reply after joining when `no-implicit-names` is enabled.
    #[must_use]
    pub const fn is_synced(&self) -> bool {
        self.synced
    }

    /// Iterates over channel members in unspecified order.
    pub fn members(&self) -> impl Iterator<Item = &Member> {
        self.members.values()
    }

    /// Returns a member using the server's current casemapping.
    #[must_use]
    pub fn member(&self, nick: impl AsRef<str>) -> Option<&Member> {
        let nick = self.casemapping.fold(nick.as_ref());
        self.members.get(nick.as_ref())
    }
}

impl Member {
    /// Returns the member's current nickname.
    #[must_use]
    pub fn nick(&self) -> &Nick {
        &self.nick
    }

    /// Returns channel-prefix symbols from highest to lowest rank.
    #[must_use]
    pub fn prefixes(&self) -> &[char] {
        &self.prefixes
    }

    /// Reports whether the member has the named prefix or any higher-ranked prefix.
    #[must_use]
    pub fn has_prefix_at_least(&self, prefix: char) -> bool {
        self.prefix_order
            .iter()
            .position(|item| *item == prefix)
            .is_some_and(|required| {
                self.prefixes.iter().any(|owned| {
                    self.prefix_order
                        .iter()
                        .position(|item| item == owned)
                        .is_some_and(|rank| rank <= required)
                })
            })
    }

    /// Reports whether the member has operator rank or higher.
    #[must_use]
    pub fn is_op(&self) -> bool {
        self.has_prefix_at_least('@')
    }

    /// Reports whether the member has the voice prefix.
    #[must_use]
    pub fn is_voiced(&self) -> bool {
        self.prefixes.contains(&'+')
    }

    /// Reports whether the member has half-operator rank or higher.
    #[must_use]
    pub fn is_halfop(&self) -> bool {
        self.has_prefix_at_least('%')
    }

    /// Returns the member's highest-ranked prefix.
    #[must_use]
    pub fn highest_prefix(&self) -> Option<char> {
        self.prefixes.first().copied()
    }
}

impl User {
    fn new(nick: &str, user: Option<&str>, host: Option<&str>, casemapping: Casemapping) -> Self {
        Self {
            nick: casemapping.nick(nick),
            username: user.map(str::to_owned),
            host: host.map(str::to_owned),
            account: Account::Unknown,
            away: Away::Present,
            realname: None,
            is_bot: false,
            monitored: false,
            channels: HashSet::new(),
        }
    }

    fn update_userhost(&mut self, user: Option<&str>, host: Option<&str>) {
        if let Some(user) = user {
            self.username = Some(user.to_owned());
        }
        if let Some(host) = host {
            self.host = Some(host.to_owned());
        }
    }

    /// Returns the user's current nickname.
    #[must_use]
    pub fn nick(&self) -> &Nick {
        &self.nick
    }

    /// Returns the user's username when known.
    #[must_use]
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// Returns the user's hostname when known.
    #[must_use]
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// Returns the user's known account state.
    #[must_use]
    pub const fn account(&self) -> &Account {
        &self.account
    }

    /// Returns the user's known presence state.
    #[must_use]
    pub const fn away(&self) -> &Away {
        &self.away
    }

    /// Returns the user's real name when known.
    #[must_use]
    pub fn realname(&self) -> Option<&str> {
        self.realname.as_deref()
    }

    /// Returns whether the user is known to be a bot.
    #[must_use]
    pub const fn is_bot(&self) -> bool {
        self.is_bot
    }

    /// Iterates over shared channel names in unspecified order.
    pub fn channels(&self) -> impl Iterator<Item = &ChannelName> {
        self.channels.iter().map(Folded::value)
    }
}

impl Account {
    fn from_notify(account: Option<&str>, casemapping: Casemapping) -> Self {
        account.map_or(Self::LoggedOut, |account| {
            Self::LoggedIn(casemapping.account(account))
        })
    }
}

fn parse_unix_seconds(value: &str) -> Option<SystemTime> {
    value
        .parse()
        .ok()
        .and_then(|seconds| UNIX_EPOCH.checked_add(Duration::from_secs(seconds)))
}

#[cfg(test)]
mod metadata_tests {
    use papo_proto::LineRef;

    use super::State;
    use crate::{Config, Event, Outbox, StateLimit};

    #[test]
    fn retains_at_most_ten_thousand_metadata_targets() {
        let mut state = State::new(&Config::default());
        let mut outbox = Outbox::default();
        for index in 0..10_000 {
            state.update_metadata(
                &format!("user{index}"),
                "avatar",
                "*",
                Some("https://example.test/a.png"),
                100,
                &mut outbox,
            );
            outbox.events.clear();
        }
        state.update_metadata("overflow", "avatar", "*", Some("x"), 100, &mut outbox);
        assert_eq!(state.metadata_target_count(), 10_000);
        assert!(matches!(
            outbox.events.front(),
            Some(Event::StateLimitReached {
                limit: StateLimit::MetadataTargets,
            })
        ));
    }

    #[test]
    fn cache_updates_bound_comparisons_to_affected_targets() {
        let state = State::new(&Config::default());

        let monitor =
            LineRef::parse(":ergo.test 730 papo :alice!alice@users.example,bob!bob@users.example")
                .unwrap();
        assert_eq!(state.snapshot_for(&monitor).comparison_steps(), Some(2));

        let metadata =
            LineRef::parse(":soju.example METADATA #papo vendor.example/pinned * :1").unwrap();
        assert_eq!(state.snapshot_for(&metadata).comparison_steps(), Some(1));

        #[cfg(feature = "unstable")]
        {
            let network = LineRef::parse(
                "BOUNCER NETWORK network-42 host=irc.example.org;state=disconnected",
            )
            .unwrap();
            assert_eq!(state.snapshot_for(&network).comparison_steps(), Some(1));
        }
    }
}
