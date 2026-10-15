use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use papo_proto::{
    CaseKey, Casemapping, Command, Isupport, Join, MessageRef, NamesEntry, Numeric, Source,
    parse_353,
};

use crate::{Config, Event, Handled, Outbox, StateLimit};

/// The known channel and user state for a session.
#[derive(Debug)]
pub struct State {
    nick: String,
    user: String,
    host: Option<String>,
    casemapping: Casemapping,
    channels: HashMap<CaseKey, Channel>,
    users: HashMap<CaseKey, User>,
    max_channels: usize,
    max_members_per_channel: usize,
    channel_limit_warned: bool,
    member_limit_warned: bool,
}

/// State known about a joined channel.
#[derive(Debug)]
pub struct Channel {
    name: String,
    casemapping: Casemapping,
    members: HashMap<CaseKey, Member>,
    topic: Option<String>,
    synced: bool,
    prefix_order: Arc<[char]>,
}

/// State known about a channel member.
#[derive(Debug)]
pub struct Member {
    nick: String,
    prefixes: Vec<char>,
    prefix_order: Arc<[char]>,
}

/// State known about a user sharing at least one channel with the session.
#[derive(Debug)]
pub struct User {
    nick: String,
    username: Option<String>,
    host: Option<String>,
    channels: HashSet<CaseKey>,
}

impl State {
    pub(crate) fn new(config: &Config) -> Self {
        Self {
            nick: config.nick.clone(),
            user: config.username.clone(),
            host: None,
            casemapping: Casemapping::default(),
            channels: HashMap::new(),
            users: HashMap::new(),
            max_channels: config.max_channels,
            max_members_per_channel: config.max_members_per_channel,
            channel_limit_warned: false,
            member_limit_warned: false,
        }
    }

    pub(crate) fn reset(&mut self, config: &Config) {
        *self = Self::new(config);
    }

    pub(crate) fn handle_message(
        &mut self,
        message: &MessageRef<'_>,
        own_nick: &str,
        isupport: &Isupport,
        no_implicit_names: bool,
        outbox: &mut Outbox,
    ) -> Handled {
        let mapping = isupport.casemapping();
        self.casemapping = mapping;
        if let Some(join) = Join::parse(message) {
            let Some(Source::User { nick, user, host }) = message.source() else {
                return Handled::No;
            };
            if mapping.eq(nick, own_nick) {
                nick.clone_into(&mut self.nick);
                if let Some(user) = user {
                    user.clone_into(&mut self.user);
                }
                if let Some(host) = host {
                    self.host = Some(host.to_owned());
                }
                let key = CaseKey::new(join.channel, mapping);
                if !self.channels.contains_key(&key) && self.channels.len() >= self.max_channels {
                    self.warn_limit(StateLimit::Channels, outbox);
                    return Handled::Yes;
                }
                self.channels.insert(
                    key,
                    Channel::new(join.channel, mapping, isupport, no_implicit_names),
                );
            }
            self.add_member(join.channel, nick, user, host, Vec::new(), outbox);
            return Handled::Yes;
        }
        if message.command() == Command::Part {
            let Some(Source::User { nick, .. }) = message.source() else {
                return Handled::No;
            };
            let Some(channels) = message.param(0) else {
                return Handled::No;
            };
            for channel in channels.split(',') {
                if mapping.eq(nick, own_nick) {
                    self.remove_channel(channel);
                } else {
                    self.remove_member(channel, nick);
                }
            }
            return Handled::Yes;
        }
        if message.command() == Command::Kick {
            let (Some(channel), Some(nick)) = (message.param(0), message.param(1)) else {
                return Handled::No;
            };
            if mapping.eq(nick, own_nick) {
                self.remove_channel(channel);
            } else {
                self.remove_member(channel, nick);
            }
            return Handled::Yes;
        }
        if message.command() == Command::Quit {
            let Some(Source::User { nick, .. }) = message.source() else {
                return Handled::No;
            };
            if mapping.eq(nick, own_nick) {
                self.channels.clear();
                self.users.clear();
            } else {
                self.remove_user(nick);
            }
            return Handled::Yes;
        }
        if message.command() == Command::Nick {
            let (Some(Source::User { nick, .. }), Some(new_nick)) =
                (message.source(), message.param(0))
            else {
                return Handled::No;
            };
            self.rename_user(nick, new_nick);
            if mapping.eq(nick, own_nick) {
                new_nick.clone_into(&mut self.nick);
            }
            return Handled::Yes;
        }
        if let Some((_, channel, entries)) = parse_353(message) {
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
        if self.finish_names(message) {
            return Handled::Yes;
        }
        Handled::No
    }

    fn finish_names(&mut self, message: &MessageRef<'_>) -> bool {
        if message.command() != Command::Numeric(Numeric::RPL_ENDOFNAMES) {
            return false;
        }
        if let Some(channel) = message
            .param(1)
            .and_then(|name| self.channels.get_mut(&CaseKey::new(name, self.casemapping)))
        {
            channel.synced = true;
        }
        true
    }

    fn add_member(
        &mut self,
        channel_name: &str,
        nick: &str,
        user: Option<&str>,
        host: Option<&str>,
        mut prefixes: Vec<char>,
        outbox: &mut Outbox,
    ) {
        let mapping = self.casemapping;
        let channel_key = CaseKey::new(channel_name, mapping);
        let nick_key = CaseKey::new(nick, mapping);
        let full = self.channels.get(&channel_key).is_some_and(|channel| {
            !channel.members.contains_key(&nick_key)
                && channel.members.len() >= self.max_members_per_channel
        });
        if full {
            self.warn_limit(StateLimit::Members, outbox);
            return;
        }
        if let Some(channel) = self.channels.get_mut(&channel_key) {
            prefixes.sort_by_key(|symbol| channel.prefix_order.iter().position(|p| p == symbol));
            prefixes.dedup();
            channel.members.insert(
                nick_key,
                Member {
                    nick: nick.to_owned(),
                    prefixes,
                    prefix_order: Arc::clone(&channel.prefix_order),
                },
            );
            let shared = self
                .users
                .entry(CaseKey::new(nick, mapping))
                .or_insert_with(|| User::new(nick, user, host));
            shared.update_userhost(user, host);
            shared.channels.insert(CaseKey::new(channel_name, mapping));
        }
    }

    fn warn_limit(&mut self, limit: StateLimit, outbox: &mut Outbox) {
        let warned = match limit {
            StateLimit::Channels => &mut self.channel_limit_warned,
            StateLimit::Members => &mut self.member_limit_warned,
        };
        if !*warned {
            *warned = true;
            outbox.event(Event::StateLimitReached { limit });
        }
    }

    fn remove_channel(&mut self, name: &str) {
        let key = CaseKey::new(name, self.casemapping);
        let Some(channel) = self.channels.remove(&key) else {
            return;
        };
        for member in channel.members.keys() {
            if let Some(user) = self.users.get_mut(member) {
                user.channels.remove(&key);
            }
        }
        self.users.retain(|_, user| !user.channels.is_empty());
    }

    fn remove_member(&mut self, channel: &str, nick: &str) {
        let channel = CaseKey::new(channel, self.casemapping);
        let nick = CaseKey::new(nick, self.casemapping);
        if let Some(joined) = self.channels.get_mut(&channel) {
            joined.members.remove(&nick);
        }
        if let Some(user) = self.users.get_mut(&nick) {
            user.channels.remove(&channel);
            if user.channels.is_empty() {
                self.users.remove(&nick);
            }
        }
    }

    fn remove_user(&mut self, nick: &str) {
        let key = CaseKey::new(nick, self.casemapping);
        let Some(user) = self.users.remove(&key) else {
            return;
        };
        for channel in user.channels {
            if let Some(joined) = self.channels.get_mut(&channel) {
                joined.members.remove(&key);
            }
        }
    }

    fn rename_user(&mut self, old_nick: &str, new_nick: &str) {
        let old_key = CaseKey::new(old_nick, self.casemapping);
        let new_key = CaseKey::new(new_nick, self.casemapping);
        let Some(mut user) = self.users.remove(&old_key) else {
            return;
        };
        new_nick.clone_into(&mut user.nick);
        for channel_key in &user.channels {
            if let Some(channel) = self.channels.get_mut(channel_key)
                && let Some(mut member) = channel.members.remove(&old_key)
            {
                new_nick.clone_into(&mut member.nick);
                channel.members.insert(new_key.clone(), member);
            }
        }
        self.users.insert(new_key, user);
    }

    /// Returns the session's current nickname.
    #[must_use]
    pub fn nick(&self) -> &str {
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

    /// Returns a joined channel using the server's current casemapping.
    #[must_use]
    pub fn channel(&self, name: &str) -> Option<&Channel> {
        self.channels.get(&CaseKey::new(name, self.casemapping))
    }

    /// Iterates over joined channels in unspecified order.
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.channels.values()
    }

    /// Returns a shared user using the server's current casemapping.
    #[must_use]
    pub fn user(&self, nick: &str) -> Option<&User> {
        self.users.get(&CaseKey::new(nick, self.casemapping))
    }
}

impl Channel {
    fn new(name: &str, casemapping: Casemapping, isupport: &Isupport, synced: bool) -> Self {
        Self {
            name: name.to_owned(),
            casemapping,
            members: HashMap::new(),
            topic: None,
            synced,
            prefix_order: isupport
                .prefix()
                .into_iter()
                .map(|prefix| prefix.symbol)
                .collect(),
        }
    }

    /// Returns the channel name as received from the server.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the current topic when known.
    #[must_use]
    pub fn topic(&self) -> Option<&str> {
        self.topic.as_deref()
    }

    /// Returns whether initial member synchronization is complete.
    ///
    /// A channel is immediately synchronized when `no-implicit-names` is enabled,
    /// because that capability suppresses the automatic `353` and `366` replies.
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
    pub fn member(&self, nick: &str) -> Option<&Member> {
        self.members.get(&CaseKey::new(nick, self.casemapping))
    }
}

impl Member {
    /// Returns the member's current nickname.
    #[must_use]
    pub fn nick(&self) -> &str {
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
}

impl User {
    fn new(nick: &str, user: Option<&str>, host: Option<&str>) -> Self {
        Self {
            nick: nick.to_owned(),
            username: user.map(str::to_owned),
            host: host.map(str::to_owned),
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
    pub fn nick(&self) -> &str {
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

    /// Iterates over shared channel names in unspecified order.
    pub fn channels(&self) -> impl Iterator<Item = &str> {
        self.channels.iter().map(CaseKey::original)
    }
}
