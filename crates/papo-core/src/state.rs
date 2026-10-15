use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use papo_proto::{
    Account as AccountNotify, Away, CaseKey, Casemapping, Chghost, Command, Isupport, Join,
    MessageRef, ModeChanges, ModeKind, NamesEntry, Numeric, Setname, Source, WhoxFlags,
    WhoxPresence, WhoxReply, parse_353,
};

use crate::{Config, Event, Handled, Outbox, OwnedModeChange, StateLimit};

const WHOX_FIELDS: &str = "%tcuhnfar,152";
const WHOX_TOKEN: &str = "152";

/// The known channel and user state for a session.
#[derive(Debug)]
pub struct State {
    nick: String,
    user: String,
    host: Option<String>,
    realname: String,
    away: bool,
    user_modes: HashSet<char>,
    casemapping: Casemapping,
    channels: HashMap<CaseKey, Channel>,
    users: HashMap<CaseKey, User>,
    who_sweeps: HashSet<CaseKey>,
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
    modes: HashMap<char, Option<String>>,
    created_at: Option<u64>,
    topic: Option<String>,
    topic_set_by: Option<String>,
    topic_set_at: Option<u64>,
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
    account: Account,
    away: Option<String>,
    realname: Option<String>,
    is_bot: bool,
    channels: HashSet<CaseKey>,
}

/// A user's known account state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Account {
    /// No source has reported whether the user is logged in.
    Unknown,
    /// The user is known to be logged out.
    LoggedOut,
    /// The user is logged in to the named account.
    LoggedIn(String),
}

#[derive(Clone, Copy)]
struct WhoUser<'a> {
    channel: &'a str,
    nick: &'a str,
    username: Option<&'a str>,
    host: Option<&'a str>,
    realname: Option<&'a str>,
}

impl State {
    pub(crate) fn new(config: &Config) -> Self {
        Self {
            nick: config.nick.clone(),
            user: config.username.clone(),
            host: None,
            realname: config.realname.clone(),
            away: false,
            user_modes: HashSet::new(),
            casemapping: Casemapping::default(),
            channels: HashMap::new(),
            users: HashMap::new(),
            who_sweeps: HashSet::new(),
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
        let mapping = self.prepare_message(message, own_nick, isupport, outbox);
        if let Some(join) = Join::parse(message) {
            return self.handle_join(message, join, isupport, no_implicit_names, outbox);
        }
        if let Some(handled) = self.handle_user_update(message, isupport, outbox) {
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
                new_nick.clone_into(&mut self.nick);
            }
            outbox.event(Event::NickChanged {
                old: nick.to_owned(),
                new: new_nick.to_owned(),
            });
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
        if self.finish_names(message, isupport, outbox) {
            return Handled::Yes;
        }
        Handled::No
    }

    fn handle_channel_mode(&mut self, message: &MessageRef<'_>, isupport: &Isupport) -> bool {
        let Some(target) = message
            .param(0)
            .filter(|target| message.command() == Command::Mode && isupport.is_channel(target))
        else {
            return false;
        };
        if let Some(modestring) = message.param(1)
            && let Some(channel) = self
                .channels
                .get_mut(&CaseKey::new(target, self.casemapping))
        {
            channel.apply_modes(modestring, message.params().skip(2), isupport, false);
        }
        true
    }

    fn report_mode_change(
        message: &MessageRef<'_>,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> bool {
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

    fn handle_channel_metadata(&mut self, message: &MessageRef<'_>, isupport: &Isupport) -> bool {
        if !matches!(
            message.command(),
            Command::Numeric(Numeric::RPL_CHANNELMODEIS | Numeric::RPL_CREATIONTIME)
        ) {
            return false;
        }
        let Some(channel) = message
            .param(1)
            .and_then(|name| self.channels.get_mut(&CaseKey::new(name, self.casemapping)))
        else {
            return true;
        };
        if message.command() == Command::Numeric(Numeric::RPL_CHANNELMODEIS) {
            if let Some(modestring) = message.param(2) {
                channel.apply_modes(modestring, message.params().skip(3), isupport, true);
            }
        } else {
            channel.created_at = message.param(2).and_then(|seconds| seconds.parse().ok());
        }
        true
    }

    fn handle_user_modes(&mut self, message: &MessageRef<'_>) -> bool {
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

    fn handle_topic(&mut self, message: &MessageRef<'_>, outbox: &mut Outbox) -> bool {
        let (channel_name, topic, live) = match message.command() {
            Command::Numeric(Numeric::RPL_TOPIC) => (message.param(1), message.param(2), false),
            Command::Topic => (message.param(0), message.param(1), true),
            Command::Numeric(Numeric::RPL_TOPICWHOTIME) => {
                let Some(channel) = message
                    .param(1)
                    .and_then(|name| self.channels.get_mut(&CaseKey::new(name, self.casemapping)))
                else {
                    return true;
                };
                channel.topic_set_by = message.param(2).map(str::to_owned);
                channel.topic_set_at = message.param(3).and_then(|value| value.parse().ok());
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
            .and_then(|name| self.channels.get_mut(&CaseKey::new(name, self.casemapping)))
        else {
            return true;
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
                    .map(|member| (CaseKey::new(&member.nick, mapping), member))
                    .collect();
                (CaseKey::new(&channel.name, mapping), channel)
            })
            .collect();
        self.users = std::mem::take(&mut self.users)
            .into_values()
            .map(|mut user| {
                user.channels = user
                    .channels
                    .into_iter()
                    .map(|channel| CaseKey::new(channel.original(), mapping))
                    .collect();
                (CaseKey::new(&user.nick, mapping), user)
            })
            .collect();
        self.who_sweeps = std::mem::take(&mut self.who_sweeps)
            .into_iter()
            .map(|channel| CaseKey::new(channel.original(), mapping))
            .collect();
        self.casemapping = mapping;
    }

    fn handle_quit(&mut self, message: &MessageRef<'_>, outbox: &mut Outbox) -> Handled {
        let Some(Source::User { nick, .. }) = message.source() else {
            return Handled::No;
        };
        let ours = self.casemapping.eq(nick, &self.nick);
        let channels = if ours {
            self.channels
                .values()
                .map(|channel| channel.name.clone())
                .collect()
        } else {
            self.users
                .get(&CaseKey::new(nick, self.casemapping))
                .map(|user| {
                    user.channels
                        .iter()
                        .map(|key| key.original().to_owned())
                        .collect()
                })
                .unwrap_or_default()
        };
        outbox.event(Event::Quit {
            nick: nick.to_owned(),
            reason: message.param(0).map(str::to_owned),
            channels,
        });
        if ours {
            self.channels.clear();
            self.users.clear();
            self.who_sweeps.clear();
        } else {
            self.remove_user(nick);
        }
        Handled::Yes
    }

    fn handle_join(
        &mut self,
        message: &MessageRef<'_>,
        join: Join<'_>,
        isupport: &Isupport,
        no_implicit_names: bool,
        outbox: &mut Outbox,
    ) -> Handled {
        let Some(Source::User { nick, user, host }) = message.source() else {
            return Handled::No;
        };
        outbox.event(Event::Joined {
            channel: join.channel.to_owned(),
            nick: nick.to_owned(),
        });
        let ours = self.casemapping.eq(nick, &self.nick);
        if ours {
            nick.clone_into(&mut self.nick);
            if let Some(user) = user {
                user.clone_into(&mut self.user);
            }
            if let Some(host) = host {
                self.host = Some(host.to_owned());
            }
            if let Some(realname) = join.realname {
                realname.clone_into(&mut self.realname);
            }
            let key = CaseKey::new(join.channel, self.casemapping);
            if !self.channels.contains_key(&key) && self.channels.len() >= self.max_channels {
                self.warn_limit(StateLimit::Channels, outbox);
                return Handled::Yes;
            }
            self.channels.insert(
                key,
                Channel::new(join.channel, self.casemapping, isupport, no_implicit_names),
            );
        }
        self.add_member(join.channel, nick, user, host, Vec::new(), outbox);
        if let Some(user) = self.users.get_mut(&CaseKey::new(nick, self.casemapping))
            && join.realname.is_some()
        {
            user.account = Account::from_notify(join.account);
            user.realname = join.realname.map(str::to_owned);
        }
        if ours && no_implicit_names {
            self.start_who(join.channel, isupport, outbox);
            outbox.event(Event::ChannelSynced {
                channel: join.channel.to_owned(),
            });
        }
        Handled::Yes
    }

    fn finish_names(
        &mut self,
        message: &MessageRef<'_>,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> bool {
        if message.command() != Command::Numeric(Numeric::RPL_ENDOFNAMES) {
            return false;
        }
        let name = message.param(1);
        let joined = name
            .and_then(|name| self.channels.get_mut(&CaseKey::new(name, self.casemapping)))
            .is_some_and(|channel| {
                channel.synced = true;
                true
            });
        if let Some(channel) = name {
            if joined {
                self.start_who(channel, isupport, outbox);
            }
            outbox.event(Event::ChannelSynced {
                channel: channel.to_owned(),
            });
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

    fn apply_user_tags(&mut self, message: &MessageRef<'_>, outbox: &mut Outbox) {
        let Some(Source::User { nick, .. }) = message.source() else {
            return;
        };
        if let Some(user) = self.users.get_mut(&CaseKey::new(nick, self.casemapping)) {
            for (key, value) in message.tags() {
                match (key, value) {
                    ("account", Some(account)) => {
                        let account = Account::LoggedIn(account.into_owned());
                        if user.account != account {
                            user.account = account.clone();
                            outbox.event(Event::AccountChanged {
                                nick: nick.to_owned(),
                                account,
                            });
                        }
                    }
                    ("bot", _) => user.is_bot = true,
                    _ => {}
                }
            }
        }
    }

    fn apply_message_metadata(
        &mut self,
        message: &MessageRef<'_>,
        own_nick: &str,
        outbox: &mut Outbox,
    ) {
        if message.command() == Command::Numeric(Numeric::RPL_WELCOME) {
            own_nick.clone_into(&mut self.nick);
        }
        self.apply_user_tags(message, outbox);
    }

    fn prepare_message(
        &mut self,
        message: &MessageRef<'_>,
        own_nick: &str,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> Casemapping {
        let mapping = isupport.casemapping();
        self.update_casemapping(mapping);
        self.apply_message_metadata(message, own_nick, outbox);
        mapping
    }

    fn handle_account(&mut self, message: &MessageRef<'_>, outbox: &mut Outbox) -> Option<Handled> {
        let account = AccountNotify::parse(message)?;
        let Some(Source::User { nick, .. }) = message.source() else {
            return Some(Handled::No);
        };
        let account = Account::from_notify(account.account);
        if let Some(user) = self.users.get_mut(&CaseKey::new(nick, self.casemapping))
            && user.account != account
        {
            user.account = account.clone();
            outbox.event(Event::AccountChanged {
                nick: nick.to_owned(),
                account,
            });
        }
        Some(Handled::Yes)
    }

    fn handle_away(&mut self, message: &MessageRef<'_>, outbox: &mut Outbox) -> Option<Handled> {
        if let Some(away) = Away::parse(message) {
            let Source::User { nick, .. } = message.source()? else {
                return Some(Handled::No);
            };
            if let Some(user) = self.users.get_mut(&CaseKey::new(nick, self.casemapping)) {
                let message = away.message.map(str::to_owned);
                if user.away != message {
                    user.away.clone_from(&message);
                    outbox.event(Event::AwayChanged {
                        nick: nick.to_owned(),
                        message,
                    });
                }
            }
            return Some(Handled::Yes);
        }
        match message.command() {
            Command::Numeric(Numeric::RPL_AWAY) => {
                let (Some(nick), Some(away)) = (message.param(1), message.param(2)) else {
                    return Some(Handled::No);
                };
                if let Some(user) = self.users.get_mut(&CaseKey::new(nick, self.casemapping)) {
                    let message = Some(away.to_owned());
                    if user.away != message {
                        user.away.clone_from(&message);
                        outbox.event(Event::AwayChanged {
                            nick: nick.to_owned(),
                            message,
                        });
                    }
                }
            }
            Command::Numeric(Numeric::RPL_UNAWAY) if self.away => {
                self.away = false;
                outbox.event(Event::AwayChanged {
                    nick: self.nick.clone(),
                    message: None,
                });
            }
            Command::Numeric(Numeric::RPL_NOWAWAY) if !self.away => {
                self.away = true;
                outbox.event(Event::AwayChanged {
                    nick: self.nick.clone(),
                    message: Some(String::new()),
                });
            }
            Command::Numeric(Numeric::RPL_UNAWAY | Numeric::RPL_NOWAWAY) => {}
            _ => return None,
        }
        Some(Handled::Yes)
    }

    fn handle_user_update(
        &mut self,
        message: &MessageRef<'_>,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        self.handle_account(message, outbox)
            .or_else(|| self.handle_away(message, outbox))
            .or_else(|| self.handle_identity_change(message, outbox))
            .or_else(|| self.handle_who(message, isupport, outbox))
    }

    fn handle_identity_change(
        &mut self,
        message: &MessageRef<'_>,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        let Source::User { nick, .. } = message.source()? else {
            return None;
        };
        if let Some(change) = Chghost::parse(message) {
            if let Some(user) = self.users.get_mut(&CaseKey::new(nick, self.casemapping)) {
                user.update_userhost(Some(change.user), Some(change.host));
            }
            if self.casemapping.eq(nick, &self.nick) {
                change.user.clone_into(&mut self.user);
                self.host = Some(change.host.to_owned());
            }
            outbox.event(Event::HostChanged {
                nick: nick.to_owned(),
                user: change.user.to_owned(),
                host: change.host.to_owned(),
            });
            return Some(Handled::Yes);
        }
        let change = Setname::parse(message)?;
        if let Some(user) = self.users.get_mut(&CaseKey::new(nick, self.casemapping)) {
            user.realname = Some(change.realname.to_owned());
        }
        if self.casemapping.eq(nick, &self.nick) {
            change.realname.clone_into(&mut self.realname);
        }
        outbox.event(Event::RealnameChanged {
            nick: nick.to_owned(),
            realname: change.realname.to_owned(),
        });
        Some(Handled::Yes)
    }

    fn handle_who(
        &mut self,
        message: &MessageRef<'_>,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        if message.command() == Command::Numeric(Numeric::RPL_ENDOFWHO) {
            let channel = CaseKey::new(message.param(1)?, self.casemapping);
            return self.who_sweeps.remove(&channel).then_some(Handled::Yes);
        }
        if let Some(reply) = WhoxReply::parse(message, WHOX_FIELDS) {
            return self.handle_whox_reply(&reply, isupport, outbox);
        }
        self.handle_plain_who(message, isupport, outbox)
    }

    fn handle_whox_reply(
        &mut self,
        reply: &WhoxReply<'_>,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        let (Some(channel), Some(nick)) = (reply.channel, reply.nick) else {
            return Some(Handled::No);
        };
        if reply.token != Some(WHOX_TOKEN)
            || !self
                .who_sweeps
                .contains(&CaseKey::new(channel, self.casemapping))
        {
            return None;
        }
        let flags = reply.flags(isupport)?;
        self.apply_who_user(
            WhoUser {
                channel,
                nick,
                username: reply.user,
                host: reply.host,
                realname: reply.realname,
            },
            flags,
            outbox,
        );
        if let Some(user) = self.users.get_mut(&CaseKey::new(nick, self.casemapping)) {
            user.account = Account::from_notify(reply.account);
        }
        Some(Handled::Yes)
    }

    fn apply_who_user(&mut self, reply: WhoUser<'_>, flags: WhoxFlags, outbox: &mut Outbox) {
        self.add_member(
            reply.channel,
            reply.nick,
            reply.username,
            reply.host,
            flags.prefixes,
            outbox,
        );
        if let Some(user) = self
            .users
            .get_mut(&CaseKey::new(reply.nick, self.casemapping))
        {
            user.realname = reply.realname.map(str::to_owned);
            user.away = match flags.presence {
                Some(WhoxPresence::Gone) => Some(String::new()),
                Some(WhoxPresence::Here) => None,
                None => user.away.take(),
            };
            user.is_bot = flags.bot;
        }
    }

    fn handle_plain_who(
        &mut self,
        message: &MessageRef<'_>,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        if message.command() != Command::Numeric(Numeric::RPL_WHOREPLY) {
            return None;
        }
        let (channel, user, host, nick, raw_flags, hopcount_realname) = (
            message.param(1)?,
            message.param(2)?,
            message.param(3)?,
            message.param(5)?,
            message.param(6)?,
            message.param(7)?,
        );
        if !self
            .who_sweeps
            .contains(&CaseKey::new(channel, self.casemapping))
        {
            return None;
        }
        let realname = hopcount_realname.split_once(' ')?.1;
        let flags = WhoxFlags {
            presence: raw_flags.chars().find_map(|flag| match flag {
                'H' => Some(WhoxPresence::Here),
                'G' => Some(WhoxPresence::Gone),
                _ => None,
            }),
            oper: raw_flags.contains('*'),
            bot: raw_flags.contains('B')
                || isupport
                    .bot_mode()
                    .is_some_and(|mode| raw_flags.contains(mode)),
            prefixes: isupport
                .prefix()
                .into_iter()
                .filter_map(|prefix| raw_flags.contains(prefix.symbol).then_some(prefix.symbol))
                .collect(),
        };
        self.apply_who_user(
            WhoUser {
                channel,
                nick,
                username: Some(user),
                host: Some(host),
                realname: Some(realname),
            },
            flags,
            outbox,
        );
        Some(Handled::Yes)
    }

    fn start_who(&mut self, channel: &str, isupport: &Isupport, outbox: &mut Outbox) {
        let key = CaseKey::new(channel, self.casemapping);
        if !self.who_sweeps.insert(key) {
            return;
        }
        if isupport.whox() {
            outbox.transmit("WHO", [channel, WHOX_FIELDS]);
        } else {
            outbox.transmit("WHO", [channel]);
        }
    }

    fn remove_channel(&mut self, name: &str) {
        let key = CaseKey::new(name, self.casemapping);
        self.who_sweeps.remove(&key);
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

    /// Returns a joined channel using the server's current casemapping.
    #[must_use]
    pub fn channel(&self, name: &str) -> Option<&Channel> {
        self.channels.get(&CaseKey::new(name, self.casemapping))
    }

    /// Iterates over joined channels in unspecified order.
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.channels.values()
    }

    /// Iterates over the session's current user mode letters in unspecified order.
    pub fn user_modes(&self) -> impl Iterator<Item = char> + '_ {
        self.user_modes.iter().copied()
    }

    /// Returns a shared user using the server's current casemapping.
    #[must_use]
    pub fn user(&self, nick: &str) -> Option<&User> {
        self.users.get(&CaseKey::new(nick, self.casemapping))
    }
}

fn source_name<'a>(message: &'a MessageRef<'_>) -> Option<&'a str> {
    Some(match message.source()? {
        Source::Server(name) => name,
        Source::User { nick, .. } => nick,
    })
}

impl Channel {
    fn new(name: &str, casemapping: Casemapping, isupport: &Isupport, synced: bool) -> Self {
        Self {
            name: name.to_owned(),
            casemapping,
            members: HashMap::new(),
            modes: HashMap::new(),
            created_at: None,
            topic: None,
            topic_set_by: None,
            topic_set_at: None,
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
                    let Some(member) = self.members.get_mut(&CaseKey::new(nick, self.casemapping))
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
    pub fn name(&self) -> &str {
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

    /// Returns when the current topic was set, in seconds since the Unix epoch.
    #[must_use]
    pub const fn topic_set_at(&self) -> Option<u64> {
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

    /// Returns the channel creation time in seconds since the Unix epoch.
    #[must_use]
    pub const fn created_at(&self) -> Option<u64> {
        self.created_at
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
            account: Account::Unknown,
            away: None,
            realname: None,
            is_bot: false,
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

    /// Returns the user's known account state.
    #[must_use]
    pub const fn account(&self) -> &Account {
        &self.account
    }

    /// Returns the user's away message when they are marked away.
    #[must_use]
    pub fn away(&self) -> Option<&str> {
        self.away.as_deref()
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
    pub fn channels(&self) -> impl Iterator<Item = &str> {
        self.channels.iter().map(CaseKey::original)
    }
}

impl Account {
    fn from_notify(account: Option<&str>) -> Self {
        account.map_or(Self::LoggedOut, |account| {
            Self::LoggedIn(account.to_owned())
        })
    }
}
