use super::{
    Account, Channel, Command, Event, Handled, Isupport, Join, LineRef, Member, NameKey, Numeric,
    Outbox, ProtoSource, State, StateLimit, User,
};

impl State {
    pub(super) fn handle_quit(&mut self, message: &LineRef<'_>, outbox: &mut Outbox) -> Handled {
        let Some(ProtoSource::User { nick, .. }) = message.source() else {
            return Handled::No;
        };
        self.record_departure(nick, None);
        let ours = crate::names_equal(self.casemapping, nick, &self.nick);
        let authorship = self.nick_authorship(nick);
        let channels = if ours {
            self.channels
                .values()
                .map(|channel| channel.name.clone())
                .collect()
        } else {
            self.users
                .get(self.casemapping.fold(nick).as_ref())
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
            netsplit: None,
        });
        if !self.is_monitored(nick) {
            self.remove_metadata(nick);
        }
        if ours {
            self.channels.clear();
            let own = self.nick.clone();
            self.users.retain(|_, user| {
                user.channels.clear();
                user.nick == own
            });
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
            netjoin: None,
        });
        let ours = crate::names_equal(self.casemapping, nick, &self.nick);
        if ours {
            let old_nick = self.nick.clone();
            self.rename_user(&old_nick, nick);
            self.nick = self.casemapping.nick(nick);
            let key = NameKey::new(join.channel, self.casemapping);
            if !self.channels.contains_key(&key) && self.channels.len() >= self.max_channels {
                self.warn_limit(StateLimit::Channels, outbox);
                return Handled::Yes;
            }
            self.channels.insert(
                key,
                std::sync::Arc::new(Channel::new(
                    join.channel,
                    self.casemapping,
                    isupport,
                    false,
                )),
            );
        }
        self.add_member(join.channel, nick, username, host, &[], outbox);
        if let Some(user) = self.users.get_mut(self.casemapping.fold(nick).as_ref())
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
        let Some(channel) = name.and_then(|name| self.channel_mut(name)) else {
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
                .get(self.casemapping.fold(channel).as_ref())
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
        prefixes: &[char],
        outbox: &mut Outbox,
    ) {
        let mapping = self.casemapping;
        let channel_key = mapping.fold(channel_name);
        let nick_key = mapping.fold(nick);
        let full = self
            .channels
            .get(channel_key.as_ref())
            .is_some_and(|channel| {
                !channel.members.contains_key(nick_key.as_ref())
                    && channel.members.len() >= self.max_members_per_channel
            });
        if full {
            self.warn_limit(StateLimit::Members, outbox);
            return;
        }
        if let Some(channel) = self.channel_mut(channel_name) {
            let prefixes = channel
                .prefix_order
                .iter()
                .copied()
                .filter(|prefix| prefixes.contains(&prefix.symbol()))
                .collect();
            channel.members.insert(
                NameKey::new(nick, mapping),
                Member {
                    nick: mapping.nick(nick),
                    prefixes,
                },
            );
            let shared = self
                .users
                .entry(NameKey::new(nick, mapping))
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
        let folded = self.casemapping.fold(name);
        self.remove_metadata(name);
        self.who_sweeps.remove(&key);
        let Some(channel) = self.channels.remove(folded.as_ref()) else {
            return;
        };
        for member in channel.members.values() {
            if let Some(user) = self
                .users
                .get_mut(self.casemapping.fold(&member.nick).as_ref())
            {
                user.channels.remove(&key);
            }
        }
        let orphaned: Vec<_> = self
            .users
            .values()
            .filter(|user| {
                user.nick != self.nick && user.channels.is_empty() && !self.is_monitored(&user.nick)
            })
            .map(|user| user.nick.clone())
            .collect();
        for nick in orphaned {
            self.remove_metadata(&nick);
        }
        let own = self.nick.clone();
        self.users
            .retain(|_, user| user.nick == own || user.monitored || !user.channels.is_empty());
    }

    pub(super) fn remove_member(&mut self, channel: &str, nick: &str) {
        let channel_name = self.casemapping.channel(channel);
        let nick_name = self.casemapping.nick(nick);
        let nick_key = self.casemapping.fold(nick);
        if let Some(joined) = self.channel_mut(channel) {
            joined.members.remove(nick_key.as_ref());
        }
        if let Some(user) = self.users.get_mut(nick_key.as_ref()) {
            user.channels.remove(&channel_name);
            if user.channels.is_empty() && !user.monitored && nick_name != self.nick {
                self.users.remove(nick_key.as_ref());
            }
        }
        if self
            .users
            .get(nick_key.as_ref())
            .is_none_or(|user| user.channels.is_empty())
            && !self.is_monitored(&nick_name)
        {
            self.remove_metadata(&nick_name);
        }
    }

    pub(super) fn remove_user(&mut self, nick: &str) {
        let folded = self.casemapping.fold(nick);
        let Some(user) = self.users.remove(folded.as_ref()) else {
            return;
        };
        for channel in user.channels {
            if let Some(joined) = self.channel_mut(&channel) {
                joined.members.remove(folded.as_ref());
            }
        }
    }

    pub(super) fn rename_user(&mut self, old_nick: &str, new_nick: &str) {
        let mapping = self.casemapping;
        let old_key = mapping.fold(old_nick);
        let new_key = mapping.nick(new_nick);
        let Some(mut user) = self.users.remove(old_key.as_ref()) else {
            return;
        };
        user.nick = mapping.nick(new_nick);
        for channel_key in &user.channels {
            if let Some(channel) = self.channel_mut(channel_key)
                && let Some(mut member) = channel.members.remove(old_key.as_ref())
            {
                member.nick = mapping.nick(new_nick);
                channel
                    .members
                    .insert(NameKey::new(new_nick, mapping), member);
            }
        }
        self.users
            .insert(NameKey::new(new_key.as_ref(), mapping), user);
    }

    pub(super) fn is_monitored(&self, nick: &str) -> bool {
        let key = self.casemapping.nick(nick);
        self.monitored.contains(&key)
    }
}
