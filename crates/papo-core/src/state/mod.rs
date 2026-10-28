#[cfg(feature = "unstable")]
mod bouncer;
pub(crate) mod membership;
pub(crate) mod metadata;
#[cfg(feature = "unstable")]
pub(crate) mod persistence;
pub(crate) mod presence;
pub(crate) mod read_marker;
pub(crate) mod rename;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use papo_proto::{
    Account as AccountNotify, AccountName, AsChannel, AsNick, Away as AwayNotify, Casemapping,
    ChannelName, Chghost, Command, Isupport, Join, LineRef, Mask, MetadataKey,
    ModeChange as ProtoModeChange, ModeChanges, ModeKind, NamesEntry, Nick, Numeric, Origin,
    Prefix, Rank, Setname, Source as ProtoSource, WhoxFlags, WhoxPresence, WhoxReply, parse_353,
    time,
};

#[cfg(test)]
use crate::monitor::presence_nicks;
use crate::{
    Authorship, Capabilities, Config, Conversation, Event, Handled, LastSeen, ListMode,
    MetadataValue, ModeArg, ModeChange, ModeListEntry, ModeValue, NameKey, Names, Outbox,
    ReadMarker, Secret, Source, StateLimit, Subject, WhoOnJoin,
    bounded::BoundedCollection,
    cap,
    conversation::{LineTarget, classify_target_with_names},
    monitor::Monitor,
    typing::{Typers, TypingTable},
};
#[cfg(feature = "unstable")]
use crate::{BouncerNetwork, NetworkId, PersistenceSetting, PersistenceStatus};

const WHOX_FIELDS: &str = "%tcuhnfar,152";
const WHOX_TOKEN: &str = "152";
const MAX_LAST_SEEN_TARGETS: usize = 1_000;
const MAX_DEPARTED_IDENTITIES: usize = 1_000;
const MAX_READ_MARKERS: usize = 1_000;
const MAX_METADATA_TARGETS: usize = 10_000;
pub(crate) const MAX_METADATA_KEYS_PER_TARGET: usize = 10_000;
/// The known channel and user state for a session.
#[derive(Debug, Eq, PartialEq)]
pub struct State {
    nick: Nick,
    user_modes: HashSet<char>,
    casemapping: Names,
    isupport: Isupport,
    channels: HashMap<NameKey<String>, Arc<Channel>>,
    users: HashMap<NameKey<String>, User>,
    monitored: Vec<Nick>,
    who_sweeps: HashMap<ChannelName, usize>,
    who_on_join: WhoOnJoin,
    read_markers: HashMap<NameKey<String>, ReadMarker>,
    read_marker_order: BoundedCollection<NameKey<String>>,
    last_seen: HashMap<NameKey<String>, LastSeen>,
    last_seen_order: VecDeque<String>,
    reply_epoch: u64,
    channel_epoch: u64,
    departure_sequence: u64,
    departed_identities: VecDeque<crate::ReplyIdentity>,
    max_channels: usize,
    max_members_per_channel: usize,
    channel_limit_warned: bool,
    member_limit_warned: bool,
    #[cfg(feature = "unstable")]
    persistence: Option<PersistenceStatus>,
    #[cfg(feature = "unstable")]
    bouncer_networks: HashMap<NetworkId, BouncerNetwork>,
    metadata: HashMap<NameKey<String>, BTreeMap<MetadataKey, MetadataValue>>,
    metadata_subscriptions: Vec<MetadataKey>,
    limits_warned: HashSet<StateLimit>,
    pub(crate) typing: Arc<TypingTable>,
    change_generation: u64,
}

impl Clone for State {
    fn clone(&self) -> Self {
        #[cfg(test)]
        STATE_CLONES.set(STATE_CLONES.get() + 1);
        Self {
            nick: self.nick.clone(),
            user_modes: self.user_modes.clone(),
            casemapping: self.casemapping,
            isupport: self.isupport.clone(),
            channels: self.channels.clone(),
            users: self.users.clone(),
            monitored: self.monitored.clone(),
            who_sweeps: self.who_sweeps.clone(),
            who_on_join: self.who_on_join,
            read_markers: self.read_markers.clone(),
            read_marker_order: self.read_marker_order.clone(),
            last_seen: self.last_seen.clone(),
            last_seen_order: self.last_seen_order.clone(),
            reply_epoch: self.reply_epoch,
            channel_epoch: self.channel_epoch,
            departure_sequence: self.departure_sequence,
            departed_identities: self.departed_identities.clone(),
            max_channels: self.max_channels,
            max_members_per_channel: self.max_members_per_channel,
            channel_limit_warned: self.channel_limit_warned,
            member_limit_warned: self.member_limit_warned,
            #[cfg(feature = "unstable")]
            persistence: self.persistence,
            #[cfg(feature = "unstable")]
            bouncer_networks: self.bouncer_networks.clone(),
            metadata: self.metadata.clone(),
            metadata_subscriptions: self.metadata_subscriptions.clone(),
            limits_warned: self.limits_warned.clone(),
            typing: Arc::clone(&self.typing),
            change_generation: self.change_generation,
        }
    }
}

#[cfg(test)]
thread_local! {
    static STATE_CLONES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_clone_count() {
    STATE_CLONES.set(0);
}

#[cfg(test)]
pub(crate) fn clone_count() -> usize {
    STATE_CLONES.get()
}

/// State known about a joined channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Channel {
    name: ChannelName,
    casemapping: Names,
    members: HashMap<NameKey<String>, Member>,
    modes: HashMap<char, Option<String>>,
    key: Option<Secret>,
    lists: HashMap<ListMode, Vec<ModeListEntry>>,
    created_at: Option<SystemTime>,
    epoch: u64,
    topic: Option<String>,
    topic_set_by: Option<Source>,
    topic_set_at: Option<SystemTime>,
    names_reply_seen: bool,
    synced: bool,
    prefix_order: Arc<[Prefix]>,
}

/// State known about a channel member.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Member {
    nick: Nick,
    prefixes: Vec<Prefix>,
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
    is_bot: Option<bool>,
    monitored: bool,
    channels: BTreeSet<ChannelName>,
    reply_epoch: u64,
    departure_sequence: u64,
}

/// A read-only view of cached metadata values.
#[derive(Clone, Copy, Debug)]
pub struct MetadataView<'a> {
    values: &'a BTreeMap<MetadataKey, MetadataValue>,
}

impl<'a> MetadataView<'a> {
    /// Returns the value stored for `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&'a MetadataValue> {
        self.values.get(key)
    }

    /// Iterates over entries in metadata-key order.
    #[must_use]
    pub fn iter(
        &self,
    ) -> impl ExactSizeIterator<Item = (&'a MetadataKey, &'a MetadataValue)> + use<'a> {
        self.values.iter()
    }
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

impl Account {
    /// Returns the authenticated account name, when the user is known to be logged in.
    #[must_use]
    pub const fn name(&self) -> Option<&AccountName> {
        match self {
            Self::LoggedIn(name) => Some(name),
            Self::Unknown | Self::LoggedOut => None,
        }
    }
}

/// A user's known presence state.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Away {
    /// The user's away state has not been reported.
    Unknown,
    /// The user is known to be present.
    Present,
    /// The user is away, with a message when the server supplied one.
    #[non_exhaustive]
    Away {
        /// The away message, or `None` when a WHO reply reported only the away flag.
        message: Option<String>,
    },
}

impl Away {
    /// Returns the away message, when the user is away and the server supplied one.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Away {
                message: Some(message),
            } => Some(message),
            Self::Unknown | Self::Present | Self::Away { message: None } => None,
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __away_value(message: Option<String>) -> Self {
        Self::Away { message }
    }
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

#[cfg(test)]
#[derive(Eq, PartialEq)]
pub(crate) enum StateSnapshot {
    Full(Box<State>),
    Metadata {
        target: NameKey<String>,
        key: String,
        value: Option<MetadataValue>,
    },
    Monitor(Vec<(Nick, Option<User>)>),
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
    pub(super) fn channel_mut(&mut self, name: &str) -> Option<&mut Channel> {
        self.channels
            .get_mut(self.casemapping.fold(name).as_ref())
            .map(Arc::make_mut)
    }

    fn own_user(&self) -> &User {
        let nick = self.casemapping.fold(&self.nick);
        self.users
            .get(nick.as_ref())
            .expect("own user is always present")
    }

    pub(super) fn nick_authorship(&self, nick: &str) -> Authorship {
        if crate::names_equal(self.casemapping.casemapping(), nick, &self.nick) {
            Authorship::Own
        } else {
            Authorship::Other
        }
    }

    pub(super) fn source_authorship(&self, message: &LineRef<'_>) -> Authorship {
        match message.source() {
            Some(ProtoSource::User { nick, .. })
                if self.nick_authorship(nick) == Authorship::Own
                    || message.tag("account").is_some_and(|actor| {
                        self.account()
                            .name()
                            .is_some_and(|own| self.casemapping.account(actor.as_ref()) == *own)
                    }) =>
            {
                Authorship::Own
            }
            _ => Authorship::Other,
        }
    }

    #[cfg(test)]
    pub(crate) fn snapshot_for(&self, message: &LineRef<'_>) -> StateSnapshot {
        if let Some((target, key, _, _)) = metadata_update(message) {
            let target = NameKey::new(target, self.casemapping.casemapping());
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
                        let key = self.casemapping.nick(nick);
                        let folded = self.casemapping.fold(nick);
                        let user = self.users.get(folded.as_ref()).cloned();
                        (key, user)
                    })
                    .collect(),
            );
        }
        #[cfg(feature = "unstable")]
        if let Some(snapshot) = self.bouncer_snapshot(message) {
            return snapshot;
        }
        let mut state = self.clone();
        state.typing = TypingTable::new();
        StateSnapshot::Full(Box::new(state))
    }

    pub(crate) fn new(config: &Config) -> Self {
        Self::with_casemapping(config, config.casemapping)
    }

    fn with_casemapping(config: &Config, casemapping: Casemapping) -> Self {
        let names = Names::new(casemapping, config.origin);
        let nick = config.origin.__nick(casemapping, &config.nick);
        let mut own = User::new(&config.nick, Some(&config.username), None, names, 0, 0);
        own.account = Account::LoggedOut;
        own.away = Away::Present;
        own.realname = Some(config.realname.clone());
        let mut users = HashMap::new();
        users.insert(NameKey::new(nick.as_ref(), names), own);
        Self {
            nick,
            user_modes: HashSet::new(),
            casemapping: names,
            isupport: Isupport::default(),
            channels: HashMap::new(),
            users,
            monitored: Vec::new(),
            who_sweeps: HashMap::new(),
            who_on_join: config.who_on_join,
            read_markers: HashMap::new(),
            read_marker_order: BoundedCollection::new(MAX_READ_MARKERS),
            last_seen: HashMap::new(),
            last_seen_order: VecDeque::new(),
            reply_epoch: 0,
            channel_epoch: 0,
            departure_sequence: 0,
            departed_identities: VecDeque::new(),
            max_channels: config.max_channels,
            max_members_per_channel: config.max_members_per_channel,
            channel_limit_warned: false,
            member_limit_warned: false,
            #[cfg(feature = "unstable")]
            persistence: None,
            #[cfg(feature = "unstable")]
            bouncer_networks: HashMap::new(),
            metadata: HashMap::new(),
            metadata_subscriptions: Vec::new(),
            limits_warned: HashSet::new(),
            typing: TypingTable::new(),
            change_generation: 0,
        }
    }

    pub(crate) const fn change_generation(&self) -> u64 {
        self.change_generation
    }

    fn mark_changed(&mut self) {
        self.change_generation = self.change_generation.wrapping_add(1);
    }

    pub(crate) fn reset(&mut self, config: &Config, casemapping: Casemapping, isupport: &Isupport) {
        let last_seen = std::mem::take(&mut self.last_seen);
        let last_seen_order = std::mem::take(&mut self.last_seen_order);
        let reply_epoch = self.reply_epoch.wrapping_add(1);
        let channel_epoch = self.channel_epoch;
        *self = Self::with_casemapping(config, casemapping);
        self.reply_epoch = reply_epoch;
        self.channel_epoch = channel_epoch;
        for user in self.users.values_mut() {
            user.reply_epoch = reply_epoch;
        }
        self.isupport.clone_from(isupport);
        for seen in last_seen.into_values() {
            self.last_seen.insert(
                NameKey::new(&seen.target, self.casemapping.casemapping()),
                seen,
            );
        }
        self.last_seen_order = last_seen_order;
    }

    /// Returns the receipt position used to guard a response built outside the core.
    #[doc(hidden)]
    #[must_use]
    pub fn __reply_token(&self) -> crate::ReplyToken {
        crate::ReplyToken {
            origin: self.casemapping.origin(),
            identity: None,
        }
    }

    /// Returns the receipt identity for the current holder of `target`.
    #[doc(hidden)]
    #[must_use]
    pub fn __reply_token_for(&self, target: &str) -> crate::ReplyToken {
        crate::ReplyToken {
            origin: self.casemapping.origin(),
            identity: self.user(target).map(|user| crate::ReplyIdentity {
                epoch: user.reply_epoch,
                sequence: user.departure_sequence,
            }),
        }
    }

    /// Resolves the current nickname for a captured private-recipient identity.
    #[doc(hidden)]
    #[must_use]
    pub fn __resolve_recipient(&self, guard: &crate::RecipientGuard) -> crate::RecipientStatus {
        if guard.token.origin != self.casemapping.origin() {
            return crate::RecipientStatus::Gone;
        }
        let tracked = guard.token.identity.and_then(|expected| {
            self.users.values().find(|user| {
                expected.epoch == user.reply_epoch
                    && expected.sequence == user.departure_sequence
                    && match &guard.account {
                        Account::LoggedIn(account) => {
                            matches!(&user.account, Account::LoggedIn(actual) if actual == account)
                        }
                        Account::Unknown | Account::LoggedOut => true,
                    }
            })
        });
        let matching = tracked.or_else(|| match &guard.account {
            Account::LoggedIn(expected) => self.users.values().find(
                |user| matches!(&user.account, Account::LoggedIn(actual) if actual == expected),
            ),
            Account::Unknown | Account::LoggedOut => None,
        });
        if let Some(user) = matching {
            return crate::RecipientStatus::Current(user.nick.clone());
        }
        if guard
            .token
            .identity
            .is_some_and(|identity| self.departed_identities.contains(&identity))
        {
            return crate::RecipientStatus::Gone;
        }
        if self.user(&guard.nick).is_some() {
            crate::RecipientStatus::Gone
        } else {
            crate::RecipientStatus::Unknown
        }
    }

    /// Captures the current membership lifecycle for a channel return address.
    #[doc(hidden)]
    #[must_use]
    pub fn __channel_guard(&self, channel: &ChannelName) -> crate::ChannelGuard {
        let joined = self.channel(channel);
        crate::ChannelGuard {
            origin: self.casemapping.origin(),
            channel: channel.clone(),
            epoch: joined.map(|channel| channel.epoch),
            created_at: joined.and_then(Channel::created_at),
        }
    }

    /// Resolves a captured channel membership to its current name.
    #[doc(hidden)]
    #[must_use]
    pub fn __resolve_channel(&self, guard: &crate::ChannelGuard) -> Option<ChannelName> {
        if guard.origin != self.casemapping.origin() {
            return None;
        }
        if let Some(channel) = self
            .channels
            .values()
            .find(|channel| Some(channel.epoch) == guard.epoch)
        {
            if guard
                .created_at
                .is_some_and(|created| channel.created_at.is_some_and(|actual| actual != created))
            {
                return None;
            }
            return Some(channel.name.clone());
        }
        let channel = self.channel(&guard.channel)?;
        match (guard.created_at, channel.created_at) {
            (Some(expected), Some(actual)) if expected == actual => Some(channel.name.clone()),
            _ => None,
        }
    }

    fn new_user(&mut self, nick: &str, username: Option<&str>, host: Option<&str>) -> User {
        self.departure_sequence = self.departure_sequence.wrapping_add(1);
        User::new(
            nick,
            username,
            host,
            self.casemapping,
            self.reply_epoch,
            self.departure_sequence,
        )
    }

    pub(super) fn remember_departed(&mut self, user: &User) {
        self.departed_identities.push_back(crate::ReplyIdentity {
            epoch: user.reply_epoch,
            sequence: user.departure_sequence,
        });
        if self.departed_identities.len() > MAX_DEPARTED_IDENTITIES {
            self.departed_identities.pop_front();
        }
    }

    pub(crate) fn track_direct(
        &mut self,
        nick: &str,
        username: Option<&str>,
        host: Option<&str>,
        account: Account,
    ) -> crate::ReplyToken {
        let key = NameKey::new(nick, self.casemapping);
        if !self.users.contains_key(&key) {
            let user = self.new_user(nick, username, host);
            self.users.insert(key.clone(), user);
        }
        if let Some(user) = self.users.get_mut(&key) {
            user.update_userhost(username, host);
            user.account = account;
            user.monitored = true;
        }
        self.mark_changed();
        self.__reply_token_for(nick)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __recipient_changed(&self, target: &str, token: crate::ReplyToken) -> bool {
        if token.origin != self.casemapping.origin() {
            return true;
        }
        if let Some(expected) = token.identity {
            return self.user(target).is_some_and(|user| {
                expected.epoch != user.reply_epoch || expected.sequence != user.departure_sequence
            });
        }
        false
    }

    pub(crate) fn sync_monitored(&mut self, monitor: &Monitor, isupport: &Isupport) {
        self.update_casemapping(isupport.casemapping());
        let monitored = monitor
            .desired()
            .map(|nick| self.casemapping.nick(nick))
            .collect();
        if self.monitored != monitored {
            self.monitored = monitored;
            self.mark_changed();
        }
        for user in self.users.values_mut() {
            user.monitored = false;
        }
        for (nick, username, host) in monitor.online() {
            let key = NameKey::new(nick, self.casemapping);
            if !self.users.contains_key(&key) {
                let user = self.new_user(nick, username, host);
                self.users.insert(key.clone(), user);
            }
            if let Some(user) = self.users.get_mut(&key) {
                user.monitored = true;
            }
        }
        for nick in monitor.direct() {
            if let Some(user) = self.users.get_mut(self.casemapping.fold(nick).as_ref()) {
                user.monitored = true;
            }
        }
        let own = self.nick.clone();
        self.users
            .retain(|_, user| user.nick == own || user.monitored || !user.channels.is_empty());
    }

    pub(crate) fn update_isupport(&mut self, isupport: &Isupport) {
        self.update_casemapping(isupport.casemapping());
        self.isupport.clone_from(isupport);
        let prefix_order: Arc<[Prefix]> = isupport.prefix().into();
        for channel in self.channels.values_mut() {
            let channel = Arc::make_mut(channel);
            channel.prefix_order = Arc::clone(&prefix_order);
        }
    }

    pub(crate) fn sync_own_account(&mut self, account: Option<&str>) {
        self.users
            .get_mut(self.casemapping.fold(&self.nick).as_ref())
            .expect("own user is always present")
            .account = Account::from_notify(account, self.casemapping);
    }

    pub(crate) fn set_mode_list(
        &mut self,
        channel: &str,
        mode: ListMode,
        entries: Vec<ModeListEntry>,
    ) {
        if let Some(channel) = self.channel_mut(channel)
            && channel.lists.get(&mode) != Some(&entries)
        {
            channel.lists.insert(mode, entries);
            self.mark_changed();
        }
    }

    pub(crate) fn invalidate_mode_list(&mut self, channel: &str, mode: ListMode) {
        if self
            .channel_mut(channel)
            .is_some_and(|channel| channel.lists.remove(&mode).is_some())
        {
            self.mark_changed();
        }
    }

    pub(crate) fn sync_monitored_nick(
        &mut self,
        monitor: &Monitor,
        isupport: &Isupport,
        nick: &str,
    ) {
        self.update_casemapping(isupport.casemapping());
        let key = NameKey::new(nick, self.casemapping);
        if let Some((username, host)) = monitor.online_nick(nick, isupport) {
            if !self.users.contains_key(&key) {
                self.prepare_identity_arrival(nick);
                let user = self.new_user(nick, username, host);
                self.users.insert(key.clone(), user);
            }
            if let Some(user) = self.users.get_mut(&key) {
                user.monitored = true;
            }
        } else if let Some(user) = self.users.get_mut(&key) {
            user.monitored = false;
            if user.channels.is_empty()
                && user.nick != self.nick
                && let Some(user) = self.users.remove(&key)
            {
                self.remember_departed(&user);
            }
        }
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(crate) fn handle_message(
        &mut self,
        message: &LineRef<'_>,
        own_nick: &str,
        isupport: &Isupport,
        enabled_caps: &Capabilities,
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
        if enabled_caps.contains_key(&cap::PERSISTENCE) && self.handle_persistence(message, outbox)
        {
            return Handled::Yes;
        }
        #[cfg(feature = "unstable")]
        if enabled_caps.contains_key(&cap::BOUNCER_NETWORKS) && self.handle_bouncer(message, outbox)
        {
            return Handled::Yes;
        }
        self.remember_last_seen(message, own_nick);
        if enabled_caps.contains_key(&cap::READ_MARKER) && self.handle_read_marker(message, outbox)
        {
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
            let Some(ProtoSource::User { nick, .. }) = message.source() else {
                return Handled::No;
            };
            let Some(channels) = message.param(0) else {
                return Handled::No;
            };
            for channel in channels.split(',') {
                let authorship = self.nick_authorship(nick);
                outbox.event(Event::Parted {
                    envelope: crate::LineEnvelope::from_line(message),
                    actor: message.raw_source().unwrap_or(nick).to_owned(),
                    channel: mapping.channel(channel),
                    nick: mapping.nick(nick),
                    reason: message
                        .param(1)
                        .filter(|reason| !reason.is_empty())
                        .map(str::to_owned),
                    authorship,
                });
                if crate::names_equal(mapping, nick, &self.nick) {
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
            let authorship = self.source_authorship(message);
            let subject_authorship = self.nick_authorship(nick);
            outbox.event(Event::Kicked {
                envelope: crate::LineEnvelope::from_line(message),
                channel: mapping.channel(channel),
                subject: mapping.nick(nick),
                actor: Source::from_proto_with_names(source, mapping),
                authorship,
                subject_authorship,
                reason: message
                    .param(2)
                    .filter(|reason| !reason.is_empty())
                    .map(str::to_owned),
            });
            if crate::names_equal(mapping, nick, &self.nick) {
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
            let (Some(ProtoSource::User { nick, .. }), Some(new_nick)) =
                (message.source(), message.param(0))
            else {
                return Handled::No;
            };
            let own_rename = crate::names_equal(mapping, nick, &self.nick);
            let collides_with_own =
                !own_rename && crate::names_equal(mapping, new_nick, &self.nick);
            let authorship = self.source_authorship(message);
            let mut channels: Vec<_> = if authorship == Authorship::Own {
                self.channels
                    .values()
                    .map(|channel| channel.name.clone())
                    .collect()
            } else {
                self.users
                    .get(mapping.fold(nick).as_ref())
                    .map(|user| user.channels.iter().cloned().collect())
                    .unwrap_or_default()
            };
            channels.sort();
            if !collides_with_own {
                if own_rename || self.user(nick).is_some() || self.is_monitored(nick) {
                    self.rename_metadata(nick, new_nick);
                } else {
                    self.remove_metadata(nick);
                }
            }
            self.rename_user(nick, new_nick);
            if own_rename {
                self.nick = mapping.nick(new_nick);
            }
            outbox.event(Event::NickChanged {
                envelope: crate::LineEnvelope::from_line(message),
                actor: message.raw_source().unwrap_or(nick).to_owned(),
                old: mapping.nick(nick),
                new: mapping.nick(new_nick),
                channels,
                authorship,
            });
            return Handled::Yes;
        }
        if self.handle_channel_rename(message, outbox) {
            return Handled::Yes;
        }
        let mode_reported = self.report_mode_change(message, isupport, outbox);
        if self.handle_channel_mode(message, isupport)
            || self.handle_channel_metadata(message, isupport)
            || self.handle_user_modes(message, isupport)
            || self.handle_topic(message, outbox)
            || mode_reported
        {
            return Handled::Yes;
        }
        if let Some((_, channel, entries)) = parse_353(message) {
            let Some(state) = self.channel_mut(channel) else {
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
                        &entry.prefixes,
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
        let Some(ProtoSource::User {
            nick,
            username: Some(user),
            host: Some(host),
            ..
        }) = message.source()
        else {
            return false;
        };
        self.update_casemapping(isupport.casemapping());
        if !crate::names_equal(self.casemapping, nick, own_nick) {
            return false;
        }
        let old_nick = self.nick.clone();
        self.rename_user(&old_nick, nick);
        self.nick = self.casemapping.nick(nick);
        if let Some(own) = self
            .users
            .get_mut(self.casemapping.fold(&self.nick).as_ref())
        {
            let realname_changed = own.realname.as_deref() != Some(change.realname);
            own.update_userhost(Some(user), Some(host));
            own.realname = Some(change.realname.to_owned());
            if realname_changed {
                outbox.event(Event::RealnameChanged {
                    actor: message.raw_source().unwrap_or(nick).to_owned(),
                    nick: self.casemapping.nick(nick),
                    realname: change.realname.to_owned(),
                    authorship: self.nick_authorship(nick),
                });
            }
        }
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
            && let Some(channel) = self.channel_mut(target)
        {
            channel.apply_modes(modestring, message.params().skip(2), isupport, false);
            channel.apply_list_modes(message, modestring, isupport);
        }
        true
    }

    fn report_mode_change(
        &self,
        message: &LineRef<'_>,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> bool {
        let (Some(target), Some(modestring), Some(actor)) = (
            message
                .param(0)
                .filter(|_| message.command() == Command::Mode),
            message.param(1),
            message.source(),
        ) else {
            return false;
        };
        let changes = if isupport.is_channel(target) {
            ModeChanges::parse(modestring, message.params().skip(2), isupport)
                .map(|change| typed_mode_change(change, self.casemapping))
                .collect()
        } else {
            let defaults = Isupport::default();
            ModeChanges::parse(modestring, message.params().skip(2), &defaults)
                .map(|change| typed_mode_change(change, self.casemapping))
                .collect()
        };
        outbox.event(Event::ModeChanged {
            envelope: crate::LineEnvelope::from_line(message),
            subject: if isupport.is_channel(target) {
                Subject::Channel(self.casemapping.channel(target))
            } else {
                Subject::User(self.casemapping.nick(target))
            },
            actor: Source::from_proto_with_names(actor, self.casemapping),
            authorship: self.source_authorship(message),
            subject_authorship: match message.source() {
                _ if isupport.is_channel(target) => Authorship::Other,
                _ => self.nick_authorship(target),
            },
            own_nick: Some(self.nick.clone()),
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
        let Some(channel) = message.param(1).and_then(|name| self.channel_mut(name)) else {
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

    fn handle_user_modes(&mut self, message: &LineRef<'_>, isupport: &Isupport) -> bool {
        let modestring = match message.command() {
            Command::Mode
                if message
                    .param(0)
                    .is_some_and(|nick| crate::names_equal(self.casemapping, nick, &self.nick)) =>
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
        if let Some(bot_mode) = isupport.bot_mode() {
            self.users
                .get_mut(self.casemapping.fold(&self.nick).as_ref())
                .expect("own user is always present")
                .is_bot = Some(self.user_modes.contains(&bot_mode));
        }
        true
    }

    fn handle_topic(&mut self, message: &LineRef<'_>, outbox: &mut Outbox) -> bool {
        let mapping = self.casemapping;
        let (channel_name, topic, live) = match message.command() {
            Command::Numeric(Numeric::RPL_TOPIC) => (message.param(1), message.param(2), false),
            Command::Topic => (message.param(0), message.param(1), true),
            Command::Numeric(Numeric::RPL_TOPICWHOTIME) => {
                let Some(channel) = message.param(1).and_then(|name| self.channel_mut(name)) else {
                    return false;
                };
                channel.topic_set_by = message
                    .param(2)
                    .map(|setter| Source::from_prefix_with_names(setter, mapping));
                channel.topic_set_at = message.param(3).and_then(parse_unix_seconds);
                return true;
            }
            _ => return false,
        };
        let live_envelope = live.then(|| crate::LineEnvelope::from_line(message));
        if let (Some(envelope), Some(channel), Some(actor)) =
            (live_envelope.clone(), channel_name, message.source())
        {
            outbox.event(Event::TopicChanged {
                envelope,
                channel: self.casemapping.channel(channel),
                topic: topic.filter(|topic| !topic.is_empty()).map(str::to_owned),
                actor: Source::from_proto_with_names(actor, self.casemapping),
                authorship: self.source_authorship(message),
            });
        }
        let Some(channel) = channel_name.and_then(|name| self.channel_mut(name)) else {
            return false;
        };
        channel.topic = topic.filter(|topic| !topic.is_empty()).map(str::to_owned);
        channel.topic_set_by = if live {
            message
                .source()
                .map(|source| Source::from_proto_with_names(source, mapping))
        } else {
            None
        };
        channel.topic_set_at = live_envelope.and_then(|envelope| envelope.server_time());
        true
    }

    #[allow(clippy::too_many_lines)]
    fn update_casemapping(&mut self, mapping: Casemapping) {
        if mapping == self.casemapping.casemapping() {
            return;
        }
        TypingTable::refold(
            &mut self.typing,
            Names::new(mapping, self.casemapping.origin()),
        );
        let old_own_nick = self.nick.clone();
        self.nick = self
            .casemapping
            .origin()
            .__nick(mapping, self.nick.as_ref());
        let mut channels = HashMap::new();
        for mut channel in std::mem::take(&mut self.channels).into_values() {
            let name = {
                let channel = Arc::make_mut(&mut channel);
                channel.casemapping = Names::new(mapping, channel.casemapping.origin());
                channel.name = self
                    .casemapping
                    .origin()
                    .__channel(mapping, channel.name.as_ref());
                let mut members = HashMap::new();
                for mut member in std::mem::take(&mut channel.members).into_values() {
                    member.nick = self
                        .casemapping
                        .origin()
                        .__nick(mapping, member.nick.as_ref());
                    members.insert(NameKey::new(member.nick.as_ref(), mapping), member);
                }
                channel.members = members;
                channel.name.to_string()
            };
            channels.insert(NameKey::new(name, mapping), channel);
        }
        self.channels = channels;
        let mut users = HashMap::new();
        for mut user in std::mem::take(&mut self.users).into_values() {
            let incoming_is_own = user.nick.as_ref() == old_own_nick.as_ref();
            user.nick = self
                .casemapping
                .origin()
                .__nick(mapping, user.nick.as_ref());
            if let Account::LoggedIn(account) = &mut user.account {
                *account = self
                    .casemapping
                    .origin()
                    .__account(mapping, account.as_ref());
            }
            let mut channels = BTreeSet::new();
            for channel in std::mem::take(&mut user.channels) {
                channels.insert(
                    self.casemapping
                        .origin()
                        .__channel(mapping, channel.as_ref()),
                );
            }
            user.channels = channels;
            match users.entry(NameKey::new(user.nick.as_ref(), mapping)) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(user);
                }
                std::collections::hash_map::Entry::Occupied(mut entry) if incoming_is_own => {
                    entry.insert(user);
                }
                std::collections::hash_map::Entry::Occupied(_) => {}
            }
        }
        self.users = users;
        let mut who_sweeps = HashMap::new();
        for (channel, count) in std::mem::take(&mut self.who_sweeps) {
            who_sweeps.insert(
                self.casemapping
                    .origin()
                    .__channel(mapping, channel.as_ref()),
                count,
            );
        }
        self.who_sweeps = who_sweeps;
        let mut read_markers = HashMap::new();
        for mut marker in std::mem::take(&mut self.read_markers).into_values() {
            match &mut marker.conversation {
                Conversation::Channel(channel) => {
                    *channel = self
                        .casemapping
                        .origin()
                        .__channel(mapping, channel.as_ref());
                }
                Conversation::Private(nick) => {
                    *nick = self.casemapping.origin().__nick(mapping, nick.as_ref());
                }
            }
            let name = match &marker.conversation {
                Conversation::Channel(channel) => channel.as_ref(),
                Conversation::Private(nick) => nick.as_ref(),
            };
            read_markers.insert(NameKey::new(name, mapping), marker);
        }
        self.read_markers = read_markers;
        self.read_marker_order = BoundedCollection::new(MAX_READ_MARKERS);
        for key in self.read_markers.keys().cloned() {
            let evicted = self.read_marker_order.push_evict_oldest(key);
            debug_assert!(evicted.is_none());
        }
        let mut last_seen = HashMap::new();
        for seen in std::mem::take(&mut self.last_seen).into_values() {
            last_seen.insert(NameKey::new(&seen.target, mapping), seen);
        }
        self.last_seen = last_seen;
        let mut metadata = HashMap::new();
        for (target, values) in std::mem::take(&mut self.metadata) {
            metadata.insert(NameKey::new(target.original(), mapping), values);
        }
        self.metadata = metadata;
        self.casemapping = Names::new(mapping, self.casemapping.origin());
    }

    /// Returns the session's current nickname.
    #[must_use]
    pub fn nick(&self) -> &Nick {
        &self.nick
    }

    /// Parses a wire address using the server's advertised `STATUSMSG` and `CHANTYPES` rules.
    ///
    /// Prefixes not advertised through `STATUSMSG` are left on the address and validated there.
    /// In particular, `@#ops` is rejected when `@` is not advertised because `@` is not valid in
    /// a nickname.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is not a valid channel or nickname.
    pub fn parse_target(
        &self,
        target: impl AsRef<str>,
    ) -> Result<(Option<Prefix>, Conversation), papo_proto::InvalidIdentifier> {
        let (status, target) = classify_target_with_names(
            target.as_ref(),
            None,
            &self.nick,
            self.casemapping,
            &self.isupport,
        )?;
        match target {
            LineTarget::Conversation(conversation) => Ok((status, conversation)),
            LineTarget::Server => Err(papo_proto::InvalidIdentifier::InvalidCharacter('*')),
            LineTarget::Broadcast => Err(papo_proto::InvalidIdentifier::InvalidPrefix('$')),
        }
    }

    /// Builds a conversation using the server's advertised name rules.
    ///
    /// # Errors
    ///
    /// Returns an error when `text` is not a valid conversation name or includes a
    /// `STATUSMSG` prefix.
    pub fn conversation(
        &self,
        text: impl AsRef<str>,
    ) -> Result<Conversation, papo_proto::InvalidIdentifier> {
        let text = text.as_ref();
        let (status, conversation) = self.parse_target(text)?;
        match status.map(Prefix::symbol) {
            Some(prefix) => Err(papo_proto::InvalidIdentifier::InvalidPrefix(prefix)),
            None => Ok(conversation),
        }
    }

    /// Returns the rules attached to names in this state.
    #[must_use]
    pub const fn casemapping(&self) -> Casemapping {
        self.casemapping.casemapping()
    }

    /// Returns the session's username.
    #[must_use]
    pub fn username(&self) -> &str {
        self.own_user().username.as_deref().unwrap_or_default()
    }

    /// Returns the session's hostname when known.
    #[must_use]
    pub fn host(&self) -> Option<&str> {
        self.own_user().host.as_deref()
    }

    /// Returns the session's real name.
    #[must_use]
    pub fn realname(&self) -> &str {
        self.own_user().realname.as_deref().unwrap_or_default()
    }

    /// Returns the session's account state.
    #[must_use]
    pub fn account(&self) -> &Account {
        &self.own_user().account
    }

    /// Returns the session's presence state.
    #[must_use]
    pub fn away(&self) -> &Away {
        &self.own_user().away
    }

    /// Returns a joined channel using the server's current casemapping.
    ///
    /// A typed channel carrying a different client origin returns `None`.
    #[must_use]
    pub fn channel(&self, name: impl AsChannel) -> Option<&Channel> {
        let name = AsChannel::__parts(&name);
        if name.origin() != Origin::NONE && name.origin() != self.casemapping.origin() {
            return None;
        }
        self.channels
            .get(name.__folded(self.casemapping()).as_ref())
            .map(Arc::as_ref)
    }

    /// Iterates over joined channels in unspecified order.
    #[must_use]
    pub fn channels(&self) -> impl ExactSizeIterator<Item = &Channel> {
        self.channels.values().map(Arc::as_ref)
    }

    /// Reports whether two masks identify the same server list entry.
    #[must_use]
    pub fn same_list_entry(&self, a: &Mask, b: impl AsRef<str>) -> bool {
        same_list_entry(a, &self.casemapping.mask(b.as_ref()), &self.isupport)
    }

    /// Iterates over upstream networks reported by the bouncer.
    #[cfg(feature = "unstable")]
    pub fn bouncer_networks(&self) -> impl Iterator<Item = &BouncerNetwork> {
        self.bouncer_networks.values()
    }

    /// Iterates over the session's current user mode letters in unspecified order.
    pub fn user_modes(&self) -> impl Iterator<Item = char> + '_ {
        self.user_modes.iter().copied()
    }

    /// Returns a shared user using the server's current casemapping.
    ///
    /// A typed nickname carrying a different client origin returns `None`.
    #[must_use]
    pub fn user(&self, nick: impl AsNick) -> Option<&User> {
        let nick = AsNick::__parts(&nick);
        if nick.origin() != Origin::NONE && nick.origin() != self.casemapping.origin() {
            return None;
        }
        self.users.get(nick.__folded(self.casemapping()).as_ref())
    }

    /// Returns cached metadata using the server's current casemapping.
    ///
    /// User metadata fetched without a shared channel or `MONITOR` relationship is a snapshot.
    /// It is discarded when a visible `NICK` or `QUIT` invalidates that identity.
    #[must_use]
    pub fn metadata(&self, target: impl AsRef<str>) -> Option<MetadataView<'_>> {
        let target = self.casemapping.fold(target.as_ref());
        self.metadata
            .get(target.as_ref())
            .map(|values| MetadataView { values })
    }

    /// Returns the number of users and channels retained in the metadata cache.
    #[must_use]
    pub fn metadata_target_count(&self) -> usize {
        self.metadata.len()
    }

    /// Returns metadata keys confirmed in the server-side subscription list.
    #[must_use]
    pub fn metadata_subscriptions(&self) -> impl ExactSizeIterator<Item = &MetadataKey> {
        self.metadata_subscriptions.iter()
    }

    pub(crate) fn clear_metadata_subscriptions(&mut self) {
        self.metadata_subscriptions.clear();
    }

    /// Returns the newest read marker reported for a conversation.
    #[must_use]
    pub fn read_marker(&self, target: impl AsRef<str>) -> Option<&ReadMarker> {
        let target = self.casemapping.fold(target.as_ref());
        self.read_markers.get(target.as_ref())
    }

    /// Returns the newest live message remembered for a conversation.
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

    /// Iterates over desired monitored nicknames in insertion order.
    ///
    /// The iterator includes nicknames that the server rejected because its
    /// monitor list was full.
    #[must_use]
    pub fn monitored(&self) -> impl ExactSizeIterator<Item = &Nick> {
        self.monitored.iter()
    }

    /// Iterates over users currently typing in `conversation`, in start order.
    #[must_use]
    pub fn typing(&self, conversation: &Conversation) -> Typers<'_> {
        self.typing.get(Some(conversation.clone()))
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __typing_table(&self) -> &Arc<TypingTable> {
        &self.typing
    }
}

fn typed_mode_change(change: ProtoModeChange<'_>, names: Names) -> ModeChange {
    let arg = change.arg.map(|arg| match (change.mode, change.kind) {
        ('k', _) => ModeArg::Key(Secret::from(arg)),
        (_, ModeKind::Prefix(prefix)) => ModeArg::Member {
            nick: names.nick(arg),
            prefix,
        },
        (_, ModeKind::List) => ModeArg::Mask(names.mask(arg)),
        _ => ModeArg::Value(arg.to_owned()),
    });
    ModeChange {
        mode: change.mode,
        action: change.action,
        arg,
    }
}

impl Channel {
    fn new(name: &str, casemapping: Names, isupport: &Isupport, synced: bool, epoch: u64) -> Self {
        Self {
            name: casemapping.channel(name),
            casemapping,
            members: HashMap::new(),
            modes: HashMap::new(),
            key: None,
            lists: HashMap::new(),
            created_at: None,
            epoch,
            topic: None,
            topic_set_by: None,
            topic_set_at: None,
            names_reply_seen: false,
            synced,
            prefix_order: isupport.prefix().into(),
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
            self.key = None;
        }
        for change in ModeChanges::parse(modestring, args, isupport) {
            if change.mode == 'k' {
                if change.action == papo_proto::ModeAction::Add {
                    if let Some(key) = change.arg {
                        self.key = Some(Secret::from(key));
                    }
                } else {
                    self.key = None;
                }
                continue;
            }
            match change.kind {
                ModeKind::Prefix(prefix) => {
                    let Some(nick) = change.arg else {
                        continue;
                    };
                    let Some(member) = self.members.get_mut(self.casemapping.fold(nick).as_ref())
                    else {
                        continue;
                    };
                    if change.action == papo_proto::ModeAction::Add
                        && !member.prefixes.contains(&prefix)
                    {
                        member.prefixes.push(prefix);
                        for index in 1..member.prefixes.len() {
                            let mut index = index;
                            while index > 0 {
                                let Some((current, previous)) = member
                                    .prefixes
                                    .get(index)
                                    .zip(member.prefixes.get(index - 1))
                                else {
                                    break;
                                };
                                let out_of_order = self
                                    .prefix_order
                                    .iter()
                                    .position(|item| item == current)
                                    < self.prefix_order.iter().position(|item| item == previous);
                                if !out_of_order {
                                    break;
                                }
                                member.prefixes.swap(index, index - 1);
                                index -= 1;
                            }
                        }
                    } else if change.action == papo_proto::ModeAction::Remove {
                        member.prefixes.retain(|held| *held != prefix);
                    }
                }
                ModeKind::AlwaysArg | ModeKind::ArgWhenSet | ModeKind::NoArg => {
                    if change.action == papo_proto::ModeAction::Add
                        && (change.arg.is_some() || change.kind == ModeKind::NoArg)
                    {
                        if self.modes.len() < 64 || self.modes.contains_key(&change.mode) {
                            self.modes
                                .insert(change.mode, change.arg.map(str::to_owned));
                        }
                    } else if change.action == papo_proto::ModeAction::Remove {
                        self.modes.remove(&change.mode);
                    }
                }
                ModeKind::List | ModeKind::Unknown => {}
            }
        }
    }

    fn apply_list_modes(&mut self, message: &LineRef<'_>, modestring: &str, isupport: &Isupport) {
        let actor = message
            .source()
            .map(|source| Source::from_proto_with_names(source, self.casemapping));
        let set_at = crate::LineEnvelope::from_line(message).server_time();
        for change in ModeChanges::parse(modestring, message.params().skip(2), isupport) {
            let mode = match (change.kind, change.mode) {
                (ModeKind::List, 'b') => ListMode::Ban,
                (ModeKind::List, 'e') => ListMode::Except,
                (ModeKind::List, 'I') => ListMode::Invex,
                (ModeKind::List, 'q') => ListMode::Quiet,
                _ => continue,
            };
            let (Some(entries), Some(mask)) = (self.lists.get_mut(&mode), change.arg) else {
                continue;
            };
            let mask = self.casemapping.mask(mask);
            if change.action == papo_proto::ModeAction::Add {
                if !entries
                    .iter()
                    .any(|entry| same_list_entry(&entry.mask, &mask, isupport))
                {
                    entries.push(ModeListEntry {
                        mask,
                        actor: actor.clone(),
                        set_at,
                    });
                }
            } else {
                entries.retain(|entry| !same_list_entry(&entry.mask, &mask, isupport));
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
    pub const fn topic_set_by(&self) -> Option<&Source> {
        self.topic_set_by.as_ref()
    }

    /// Returns when the current topic was set.
    #[must_use]
    pub const fn topic_set_at(&self) -> Option<SystemTime> {
        self.topic_set_at
    }

    /// Returns a stored non-list mode and its value, if set.
    #[must_use]
    pub fn mode(&self, mode: char) -> Option<ModeValue<'_>> {
        if mode == 'k' {
            return self.key.as_ref().map(|_| ModeValue::Set);
        }
        self.modes
            .get(&mode)
            .map(|argument| argument.as_deref().map_or(ModeValue::Set, ModeValue::Value))
    }

    /// Iterates over stored non-list modes in unspecified order.
    pub fn modes(&self) -> impl Iterator<Item = (char, ModeValue<'_>)> {
        self.modes
            .iter()
            .map(|(mode, argument)| {
                (
                    *mode,
                    argument.as_deref().map_or(ModeValue::Set, ModeValue::Value),
                )
            })
            .chain(self.key.as_ref().map(|_| ('k', ModeValue::Set)))
    }

    /// Returns the current channel key.
    ///
    /// Only live state for a joined channel is retained. Keys from history playback, raw replies
    /// for unjoined channels, failed joins, and generic outgoing mode requests are outside this
    /// accessor's scope.
    #[must_use]
    pub const fn key(&self) -> Option<&Secret> {
        self.key.as_ref()
    }

    /// Returns a fetched list mode, or `None` while its contents are unknown.
    #[must_use]
    pub fn list(&self, mode: ListMode) -> Option<&[ModeListEntry]> {
        self.lists.get(&mode).map(Vec::as_slice)
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
    #[must_use]
    pub fn members(&self) -> impl ExactSizeIterator<Item = &Member> {
        self.members.values()
    }

    /// Returns a member using the server's current casemapping.
    ///
    /// A typed nickname carrying a different client origin returns `None`.
    #[must_use]
    pub fn member(&self, nick: impl AsNick) -> Option<&Member> {
        let nick = AsNick::__parts(&nick);
        if nick.origin() != Origin::NONE && nick.origin() != self.casemapping.origin() {
            return None;
        }
        self.members
            .get(nick.__folded(self.casemapping.casemapping()).as_ref())
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
    pub fn prefixes(&self) -> &[Prefix] {
        &self.prefixes
    }

    /// Returns the highest standard rank the member holds.
    ///
    /// Vendor-specific membership modes do not map to a standard rank.
    /// Compare with `>=` for a privilege threshold, such as
    /// `member.rank() >= Some(Rank::Op)`. `None` fails every such threshold.
    #[must_use]
    pub fn rank(&self) -> Option<Rank> {
        self.prefixes
            .iter()
            .filter_map(|prefix| prefix.rank())
            .max()
    }

    /// Reports whether the member holds the exact standard rank.
    ///
    /// For “at least op,” use `member.rank() >= Some(Rank::Op)`. A channel founder on Ergo holds
    /// only [`Rank::Owner`], so `member.has(Rank::Op)` is false for them.
    #[must_use]
    pub fn has(&self, rank: Rank) -> bool {
        self.prefixes
            .iter()
            .any(|prefix| prefix.rank() == Some(rank))
    }
}

impl User {
    fn new(
        nick: &str,
        user: Option<&str>,
        host: Option<&str>,
        casemapping: Names,
        reply_epoch: u64,
        departure_sequence: u64,
    ) -> Self {
        Self {
            nick: casemapping.nick(nick),
            username: user.map(str::to_owned),
            host: host.map(str::to_owned),
            account: Account::Unknown,
            away: Away::Unknown,
            realname: None,
            is_bot: None,
            monitored: false,
            channels: BTreeSet::new(),
            reply_epoch,
            departure_sequence,
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

    /// Returns whether the user is known to be a bot or human.
    ///
    /// `None` means WHO flags, a bot tag, and numeric 335 have not reported this fact.
    #[must_use]
    pub const fn is_bot(&self) -> Option<bool> {
        self.is_bot
    }

    /// Iterates over shared channel names in unspecified order.
    #[must_use]
    pub fn channels(&self) -> impl ExactSizeIterator<Item = &ChannelName> {
        self.channels.iter()
    }
}

impl Account {
    fn from_notify(account: Option<&str>, casemapping: Names) -> Self {
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

fn same_list_entry(a: &Mask, b: &Mask, isupport: &Isupport) -> bool {
    let account = |mask: &Mask| {
        let syntax = isupport.extban()?;
        let body = match syntax.prefix {
            Some(prefix) => mask.strip_prefix(prefix)?,
            None => mask.as_ref(),
        };
        let (kind, account) = body.split_once(':')?;
        isupport
            .accountextban()
            .contains(&kind)
            .then(|| mask.casemapping().account(account))
    };
    match (account(a), account(b)) {
        (Some(a), Some(b)) => a == b,
        (Some(_), None) | (None, Some(_)) => false,
        (None, None) => canonical_list_mask(a) == canonical_list_mask(b),
    }
}

fn canonical_list_mask(mask: &Mask) -> Mask {
    if mask.contains(['!', '@']) {
        mask.clone()
    } else {
        mask.casemapping().mask(format!("{mask}!*@*"))
    }
}

#[cfg(test)]
mod metadata_tests {
    use papo_proto::{Isupport, LineRef};

    use super::{Arc, Channel, MetadataContext, NameKey, State};
    use crate::{Authorship, Capabilities, Config, Event, ListMode, Outbox, StateLimit};

    #[test]
    fn overflow_warning_fires_once_while_input_continues() {
        let mut entries = crate::bounded::BoundedCollection::new(1_000);
        let mut warnings = Vec::new();

        for index in 0..20_000 {
            entries.push_evict_oldest(format!("user{index:05}"));
            if entries.take_overflow_warning() {
                warnings.push(index);
            }
        }

        assert_eq!(warnings, [1_000]);
        assert_eq!(entries.as_slice().len(), 1_000);
    }

    #[test]
    fn visible_host_uses_the_nick_when_the_own_username_is_missing() {
        let mut state = State::new(&Config::default().nick("papo"));
        let key = state.casemapping.fold("papo");
        state.users.get_mut(key.as_ref()).unwrap().username = None;
        let mut outbox = Outbox::default();
        let line = LineRef::parse(":ergo.test 396 papo cloak.example :is now your displayed host")
            .unwrap();

        state.handle_message(
            &line,
            "papo",
            &Isupport::default(),
            &Capabilities::default(),
            false,
            MetadataContext {
                max_keys: 100,
                subscription_batch: false,
            },
            None,
            &mut outbox,
        );

        assert!(matches!(
            outbox.events.pop_front(),
            Some(Event::HostChanged {
                username,
                authorship: Authorship::Own,
                ..
            }) if username == "papo"
        ));
    }

    #[test]
    fn distinguishes_unknown_and_fetched_empty_mode_lists() {
        let mut state = State::new(&Config::default());
        let channel = Channel::new(
            "#Papo[dev]",
            state.casemapping,
            &Isupport::default(),
            true,
            1,
        );
        state.channels.insert(
            NameKey::new("#Papo[dev]", state.casemapping),
            Arc::new(channel),
        );

        assert_eq!(
            state.channel("#papo{dev}").unwrap().list(ListMode::Ban),
            None
        );
        state
            .channel_mut("#papo{dev}")
            .unwrap()
            .lists
            .insert(ListMode::Ban, Vec::new());
        assert_eq!(
            state.channel("#Papo[dev]").unwrap().list(ListMode::Ban),
            Some([].as_slice())
        );
        state
            .channel_mut("#Papo[dev]")
            .unwrap()
            .lists
            .remove(&ListMode::Ban);
        assert_eq!(
            state.channel("#papo{dev}").unwrap().list(ListMode::Ban),
            None
        );
        state
            .channel_mut("#papo{dev}")
            .unwrap()
            .lists
            .insert(ListMode::Ban, Vec::new());
        for channel in state.channels.values_mut() {
            Arc::make_mut(channel).lists.clear();
        }
        assert_eq!(
            state.channel("#Papo[dev]").unwrap().list(ListMode::Ban),
            None
        );
    }

    #[test]
    fn cloned_state_copies_only_the_changed_channel() {
        let mut state = State::new(&Config::default());
        for name in ["#papo", "#rust"] {
            let channel = Channel::new(name, state.casemapping, &Isupport::default(), true, 1);
            state
                .channels
                .insert(NameKey::new(name, state.casemapping), Arc::new(channel));
        }
        let snapshot = state.clone();

        state.channel_mut("#papo").unwrap().topic = Some("release planning".into());

        assert!(!Arc::ptr_eq(
            &state.channels["#papo"],
            &snapshot.channels["#papo"]
        ));
        assert!(Arc::ptr_eq(
            &state.channels["#rust"],
            &snapshot.channels["#rust"]
        ));
        assert_eq!(snapshot.channel("#papo").unwrap().topic(), None);
    }

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
