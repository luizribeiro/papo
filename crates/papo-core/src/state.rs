use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use papo_proto::{
    CaseKey, Casemapping, Command, Isupport, Join, MessageRef, NamesEntry, Numeric, Source,
    parse_353,
};

use crate::{Config, Handled};

/// The known channel and user state for a session.
#[derive(Debug)]
pub struct State {
    nick: String,
    user: String,
    host: Option<String>,
    casemapping: Casemapping,
    channels: HashMap<CaseKey, Channel>,
    users: HashMap<CaseKey, User>,
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
                self.channels.insert(
                    CaseKey::new(join.channel, mapping),
                    Channel::new(join.channel, mapping, isupport),
                );
            }
            self.add_member(join.channel, nick, user, host, Vec::new());
            return Handled::Yes;
        }
        if let Some((_, channel, entries)) = parse_353(message) {
            for raw in entries {
                let entry = NamesEntry::parse(raw, isupport);
                if !entry.nick.is_empty() {
                    self.add_member(channel, entry.nick, entry.user, entry.host, entry.prefixes);
                }
            }
            return Handled::Yes;
        }
        if message.command() == Command::Numeric(Numeric::RPL_ENDOFNAMES) {
            if let Some(channel) = message
                .param(1)
                .and_then(|name| self.channels.get_mut(&CaseKey::new(name, mapping)))
            {
                channel.synced = true;
            }
            return Handled::Yes;
        }
        Handled::No
    }

    fn add_member(
        &mut self,
        channel_name: &str,
        nick: &str,
        user: Option<&str>,
        host: Option<&str>,
        mut prefixes: Vec<char>,
    ) {
        let mapping = self.casemapping;
        if let Some(channel) = self.channels.get_mut(&CaseKey::new(channel_name, mapping)) {
            prefixes.sort_by_key(|symbol| channel.prefix_order.iter().position(|p| p == symbol));
            prefixes.dedup();
            channel.members.insert(
                CaseKey::new(nick, mapping),
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
    fn new(name: &str, casemapping: Casemapping, isupport: &Isupport) -> Self {
        Self {
            name: name.to_owned(),
            casemapping,
            members: HashMap::new(),
            topic: None,
            synced: false,
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
