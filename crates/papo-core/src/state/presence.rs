use super::{
    Account, AccountNotify, Away, AwayNotify, Chghost, Command, Event, Handled, Isupport, LineRef,
    Numeric, Outbox, ProtoSource, Setname, State, WHOX_FIELDS, WHOX_TOKEN, WhoUser, WhoxFlags,
    WhoxPresence, WhoxReply,
};

impl State {
    fn apply_user_tags(&mut self, message: &LineRef<'_>, outbox: &mut Outbox) {
        let Some(ProtoSource::User { nick, .. }) = message.source() else {
            return;
        };
        let actor = message.raw_source().unwrap_or(nick).to_owned();
        if let Some(user) = self.users.get_mut(&self.casemapping.nick(nick)) {
            for (key, value) in message.tags() {
                match (key, value) {
                    ("account", Some(account)) => {
                        let account = Account::LoggedIn(self.casemapping.account(account.as_ref()));
                        if user.account != account {
                            user.account = account.clone();
                            outbox.event(Event::AccountChanged {
                                actor: actor.clone(),
                                nick: self.casemapping.nick(nick),
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
        message: &LineRef<'_>,
        own_nick: &str,
        outbox: &mut Outbox,
    ) {
        if message.command() == Command::Numeric(Numeric::RPL_WELCOME) {
            self.nick = self.casemapping.nick(own_nick);
        }
        self.apply_user_tags(message, outbox);
    }

    pub(super) fn prepare_message(
        &mut self,
        message: &LineRef<'_>,
        own_nick: &str,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> crate::Names {
        let mapping = isupport.casemapping();
        self.update_casemapping(mapping);
        self.apply_message_metadata(message, own_nick, outbox);
        self.casemapping
    }

    fn handle_account(&mut self, message: &LineRef<'_>, outbox: &mut Outbox) -> Option<Handled> {
        let account = AccountNotify::parse(message)?;
        let Some(ProtoSource::User { nick, .. }) = message.source() else {
            return Some(Handled::No);
        };
        let account = Account::from_notify(account.account, self.casemapping);
        let actor = message.raw_source().unwrap_or(nick).to_owned();
        if crate::names_equal(self.casemapping, nick, &self.nick) {
            self.account = account.clone();
        }
        if let Some(user) = self.users.get_mut(&self.casemapping.nick(nick))
            && user.account != account
        {
            user.account = account.clone();
            outbox.event(Event::AccountChanged {
                actor,
                nick: self.casemapping.nick(nick),
                account,
            });
        }
        Some(Handled::Yes)
    }

    fn handle_away(
        &mut self,
        message: &LineRef<'_>,
        own_away_message: Option<&str>,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        if let Some(away) = AwayNotify::parse(message) {
            let ProtoSource::User { nick, .. } = message.source()? else {
                return Some(Handled::No);
            };
            let next = match away.message {
                None => Away::Present,
                Some("*") => Away::Away { message: None },
                Some(message) => Away::Away {
                    message: Some(message.to_owned()),
                },
            };
            let own_changed =
                crate::names_equal(self.casemapping, nick, &self.nick) && self.away != next;
            if own_changed {
                self.away = next.clone();
            }
            let user_changed = self
                .users
                .get_mut(&self.casemapping.nick(nick))
                .is_some_and(|user| {
                    if user.away == next {
                        false
                    } else {
                        user.away = next.clone();
                        true
                    }
                });
            if own_changed || user_changed {
                outbox.event(Event::AwayChanged {
                    actor: message.raw_source().unwrap_or(nick).to_owned(),
                    nick: self.casemapping.nick(nick),
                    away: next,
                });
            }
            return Some(Handled::Yes);
        }
        match message.command() {
            Command::Numeric(Numeric::RPL_AWAY) => {
                let (Some(nick), Some(away)) = (message.param(1), message.param(2)) else {
                    return Some(Handled::No);
                };
                if let Some(user) = self.users.get_mut(&self.casemapping.nick(nick)) {
                    let message = away.to_owned();
                    let next = Away::Away {
                        message: Some(message.clone()),
                    };
                    if user.away != next {
                        user.away = next.clone();
                        outbox.event(Event::AwayChanged {
                            actor: nick.to_owned(),
                            nick: self.casemapping.nick(nick),
                            away: next,
                        });
                    }
                }
            }
            Command::Numeric(Numeric::RPL_UNAWAY) if self.away != Away::Present => {
                self.away = Away::Present;
                outbox.event(Event::AwayChanged {
                    actor: self.nick.to_string(),
                    nick: self.nick.clone(),
                    away: Away::Present,
                });
            }
            Command::Numeric(Numeric::RPL_NOWAWAY) if !matches!(self.away, Away::Away { .. }) => {
                self.away = Away::Away {
                    message: own_away_message.map(str::to_owned),
                };
                outbox.event(Event::AwayChanged {
                    actor: self.nick.to_string(),
                    nick: self.nick.clone(),
                    away: self.away.clone(),
                });
            }
            Command::Numeric(Numeric::RPL_UNAWAY | Numeric::RPL_NOWAWAY) => {}
            _ => return None,
        }
        Some(Handled::Yes)
    }

    pub(super) fn handle_user_update(
        &mut self,
        message: &LineRef<'_>,
        isupport: &Isupport,
        own_away_message: Option<&str>,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        self.handle_visible_host(message, outbox)
            .or_else(|| self.handle_account(message, outbox))
            .or_else(|| self.handle_away(message, own_away_message, outbox))
            .or_else(|| self.handle_identity_change(message, outbox))
            .or_else(|| self.handle_who(message, isupport, outbox))
    }

    fn handle_visible_host(
        &mut self,
        message: &LineRef<'_>,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        if message.command() != Command::Numeric(Numeric::RPL_VISIBLEHOST) {
            return None;
        }
        let (Some(nick), Some(host)) = (message.param(0), message.param(1)) else {
            return Some(Handled::No);
        };
        if !crate::names_equal(self.casemapping, nick, &self.nick) {
            return Some(Handled::No);
        }
        self.host = Some(host.to_owned());
        outbox.event(Event::HostChanged {
            actor: self.nick.to_string(),
            nick: self.nick.clone(),
            username: self.user.clone(),
            host: host.to_owned(),
        });
        Some(Handled::Yes)
    }

    fn handle_identity_change(
        &mut self,
        message: &LineRef<'_>,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        let ProtoSource::User { nick, .. } = message.source()? else {
            return None;
        };
        if let Some(change) = Chghost::parse(message) {
            if let Some(user) = self.users.get_mut(&self.casemapping.nick(nick)) {
                user.update_userhost(Some(change.user), Some(change.host));
            }
            if crate::names_equal(self.casemapping, nick, &self.nick) {
                change.user.clone_into(&mut self.user);
                self.host = Some(change.host.to_owned());
            }
            outbox.event(Event::HostChanged {
                actor: message.raw_source().unwrap_or(nick).to_owned(),
                nick: self.casemapping.nick(nick),
                username: change.user.to_owned(),
                host: change.host.to_owned(),
            });
            return Some(Handled::Yes);
        }
        let change = Setname::parse(message)?;
        if let Some(user) = self.users.get_mut(&self.casemapping.nick(nick)) {
            user.realname = Some(change.realname.to_owned());
        }
        if crate::names_equal(self.casemapping, nick, &self.nick) {
            change.realname.clone_into(&mut self.realname);
        }
        outbox.event(Event::RealnameChanged {
            actor: message.raw_source().unwrap_or(nick).to_owned(),
            nick: self.casemapping.nick(nick),
            realname: change.realname.to_owned(),
        });
        Some(Handled::Yes)
    }

    fn handle_who(
        &mut self,
        message: &LineRef<'_>,
        isupport: &Isupport,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        if message.command() == Command::Numeric(Numeric::RPL_ENDOFWHO) {
            let channel = self.casemapping.channel(message.param(1)?);
            let count = self.who_sweeps.get_mut(&channel)?;
            *count -= 1;
            if *count == 0 {
                self.who_sweeps.remove(&channel);
            }
            outbox.event(Event::MembersRefreshed { channel });
            return Some(Handled::Yes);
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
                .contains_key(&self.casemapping.channel(channel))
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
        if let Some(user) = self.users.get_mut(&self.casemapping.nick(nick)) {
            user.account = Account::from_notify(reply.account, self.casemapping);
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
        if let Some(user) = self.users.get_mut(&self.casemapping.nick(reply.nick)) {
            user.realname = reply.realname.map(str::to_owned);
            user.away = match flags.presence {
                Some(WhoxPresence::Gone) => match &user.away {
                    Away::Away { message: Some(_) } => user.away.clone(),
                    Away::Unknown | Away::Present | Away::Away { message: None } => {
                        Away::Away { message: None }
                    }
                },
                Some(WhoxPresence::Here) => Away::Present,
                None => user.away.clone(),
            };
            user.is_bot = flags.bot;
        }
    }

    fn handle_plain_who(
        &mut self,
        message: &LineRef<'_>,
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
            .contains_key(&self.casemapping.channel(channel))
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

    pub(super) fn start_who(&mut self, channel: &str, isupport: &Isupport, outbox: &mut Outbox) {
        let key = self.casemapping.channel(channel);
        if self.who_sweeps.contains_key(&key) {
            return;
        }
        self.who_sweeps.insert(key, 1);
        if isupport.whox() {
            outbox.transmit("WHO", [channel, WHOX_FIELDS]);
        } else {
            outbox.transmit("WHO", [channel]);
        }
    }

    pub(crate) fn track_who(&mut self, channel: &str) {
        let key = self.casemapping.channel(channel);
        if self.channels.contains_key(&key) {
            *self.who_sweeps.entry(key).or_default() += 1;
        }
    }
}
