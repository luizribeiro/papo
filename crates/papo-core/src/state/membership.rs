use super::{
    Account, Arc, Channel, Command, Event, Handled, Isupport, Join, LineRef, Member, Numeric,
    Outbox, ProtoSource, State, StateLimit, User,
};

impl State {
    pub(super) fn handle_quit(&mut self, message: &LineRef<'_>, outbox: &mut Outbox) -> Handled {
        let Some(ProtoSource::User { nick, .. }) = message.source() else {
            return Handled::No;
        };
        let ours = crate::names_equal(self.casemapping, nick, &self.nick);
        let authorship = self.nick_authorship(nick);
        let channels = if ours {
            self.channels
                .values()
                .map(|channel| channel.name.clone())
                .collect()
        } else {
            self.users
                .get(&self.casemapping.nick(nick))
                .map(|user| user.channels.iter().cloned().collect())
                .unwrap_or_default()
        };
        outbox.event(Event::Quit {
            envelope: crate::LineEnvelope::from_line(message),
            actor: message.raw_source().unwrap_or(nick).to_owned(),
            nick: self.casemapping.nick(nick),
            reason: message
                .param(0)
                .filter(|reason| !reason.is_empty())
                .map(str::to_owned),
            channels,
            authorship,
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

    pub(super) fn handle_join(
        &mut self,
        message: &LineRef<'_>,
        join: Join<'_>,
        isupport: &Isupport,
        no_implicit_names: bool,
        outbox: &mut Outbox,
    ) -> Handled {
        let Some(ProtoSource::User {
            nick,
            username,
            host,
            ..
        }) = message.source()
        else {
            return Handled::No;
        };
        let account = if join.realname.is_some() {
            Account::from_notify(join.account, self.casemapping)
        } else {
            Account::Unknown
        };
        outbox.event(Event::Joined {
            envelope: crate::LineEnvelope::from_line(message),
            actor: message
                .raw_source()
                .expect("user source is present")
                .to_owned(),
            account,
            realname: join.realname.map(str::to_owned),
            channel: self.casemapping.channel(join.channel),
            nick: self.casemapping.nick(nick),
            authorship: self.nick_authorship(nick),
        });
        let ours = crate::names_equal(self.casemapping, nick, &self.nick);
        if ours {
            self.nick = self.casemapping.nick(nick);
            if let Some(username) = username {
                username.clone_into(&mut self.user);
            }
            if let Some(host) = host {
                self.host = Some(host.to_owned());
            }
            if let Some(realname) = join.realname {
                realname.clone_into(&mut self.realname);
            }
            let key = self.casemapping.channel(join.channel);
            if !self.channels.contains_key(&key) && self.channels.len() >= self.max_channels {
                self.warn_limit(StateLimit::Channels, outbox);
                return Handled::Yes;
            }
            self.channels.insert(
                key,
                Channel::new(join.channel, self.casemapping, isupport, false),
            );
        }
        self.add_member(join.channel, nick, username, host, Vec::new(), outbox);
        if let Some(user) = self.users.get_mut(&self.casemapping.nick(nick))
            && join.realname.is_some()
        {
            user.account = Account::from_notify(join.account, self.casemapping);
            user.realname = join.realname.map(str::to_owned);
        }
        if ours && no_implicit_names {
            outbox.transmit("NAMES", [join.channel]);
        }
        Handled::Yes
    }

    pub(super) fn finish_names(
        &mut self,
        message: &LineRef<'_>,
        isupport: &Isupport,
        no_implicit_names: bool,
        outbox: &mut Outbox,
    ) -> bool {
        if message.command() != Command::Numeric(Numeric::RPL_ENDOFNAMES) {
            return false;
        }
        let name = message.param(1);
        let Some(channel) =
            name.and_then(|name| self.channels.get_mut(&self.casemapping.channel(name)))
        else {
            return false;
        };
        if channel.synced {
            return true;
        }
        if no_implicit_names && !channel.names_reply_seen {
            return true;
        }
        channel.synced = true;
        if let Some(channel) = name {
            outbox.transmit("MODE", [channel]);
            let members = self
                .channels
                .get(&self.casemapping.channel(channel))
                .map_or(0, |channel| channel.members.len());
            let refresh = match self.who_on_join {
                super::WhoOnJoin::Always => true,
                super::WhoOnJoin::Never => false,
                super::WhoOnJoin::BelowMembers(limit) => members < limit,
            };
            if refresh {
                self.start_who(channel, isupport, outbox);
            }
            outbox.event(Event::ChannelSynced {
                channel: self.casemapping.channel(channel),
            });
        }
        true
    }

    pub(super) fn add_member(
        &mut self,
        channel_name: &str,
        nick: &str,
        user: Option<&str>,
        host: Option<&str>,
        mut prefixes: Vec<char>,
        outbox: &mut Outbox,
    ) {
        let mapping = self.casemapping;
        let channel_key = mapping.channel(channel_name);
        let nick_key = mapping.nick(nick);
        let full = self.channels.get(&channel_key).is_some_and(|channel| {
            !channel.members.contains_key(&nick_key)
                && channel.members.len() >= self.max_members_per_channel
        });
        if full {
            self.warn_limit(StateLimit::Members, outbox);
            return;
        }
        if let Some(channel) = self.channels.get_mut(&channel_key) {
            prefixes.sort_by_key(|symbol| {
                channel
                    .prefix_order
                    .iter()
                    .position(|prefix| prefix.symbol == *symbol)
            });
            prefixes.dedup();
            channel.members.insert(
                nick_key,
                Member {
                    nick: mapping.nick(nick),
                    prefixes,
                    prefix_order: Arc::clone(&channel.prefix_order),
                },
            );
            let shared = self
                .users
                .entry(mapping.nick(nick))
                .or_insert_with(|| User::new(nick, user, host, mapping));
            shared.update_userhost(user, host);
            shared.channels.insert(mapping.channel(channel_name));
        }
    }

    pub(crate) fn warn_limit(&mut self, limit: StateLimit, outbox: &mut Outbox) {
        let first = match limit {
            StateLimit::Channels => !std::mem::replace(&mut self.channel_limit_warned, true),
            StateLimit::Members => !std::mem::replace(&mut self.member_limit_warned, true),
            StateLimit::ListEntries
            | StateLimit::MetadataTargets
            | StateLimit::MetadataKeys
            | StateLimit::IsupportTokens => self.limits_warned.insert(limit),
            #[cfg(feature = "unstable")]
            StateLimit::BouncerNetworks => self.limits_warned.insert(limit),
        };
        if first {
            outbox.event(Event::StateLimitReached { limit });
        }
    }

    pub(super) fn remove_channel(&mut self, name: &str) {
        let key = self.casemapping.channel(name);
        self.who_sweeps.remove(&key);
        let Some(channel) = self.channels.remove(&key) else {
            return;
        };
        for member in channel.members.keys() {
            if let Some(user) = self.users.get_mut(member) {
                user.channels.remove(&key);
            }
        }
        self.users
            .retain(|_, user| user.monitored || !user.channels.is_empty());
    }

    pub(super) fn remove_member(&mut self, channel: &str, nick: &str) {
        let channel = self.casemapping.channel(channel);
        let nick = self.casemapping.nick(nick);
        if let Some(joined) = self.channels.get_mut(&channel) {
            joined.members.remove(&nick);
        }
        if let Some(user) = self.users.get_mut(&nick) {
            user.channels.remove(&channel);
            if user.channels.is_empty() && !user.monitored {
                self.users.remove(&nick);
            }
        }
    }

    pub(super) fn remove_user(&mut self, nick: &str) {
        let key = self.casemapping.nick(nick);
        let Some(user) = self.users.remove(&key) else {
            return;
        };
        for channel in user.channels {
            if let Some(joined) = self.channels.get_mut(&channel) {
                joined.members.remove(&key);
            }
        }
    }

    pub(super) fn rename_user(&mut self, old_nick: &str, new_nick: &str) {
        let old_key = self.casemapping.nick(old_nick);
        let new_key = self.casemapping.nick(new_nick);
        let Some(mut user) = self.users.remove(&old_key) else {
            return;
        };
        user.nick = self.casemapping.nick(new_nick);
        for channel_key in &user.channels {
            if let Some(channel) = self.channels.get_mut(channel_key)
                && let Some(mut member) = channel.members.remove(&old_key)
            {
                member.nick = self.casemapping.nick(new_nick);
                channel.members.insert(new_key.clone(), member);
            }
        }
        self.users.insert(new_key, user);
    }
}
