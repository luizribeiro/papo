use super::{
    Account, AccountNotify, Away, AwayNotify, Casemapping, Chghost, Command, Event, Folded,
    Handled, Isupport, MessageRef, Numeric, Outbox, Setname, Source, State, WHOX_FIELDS,
    WHOX_TOKEN, WhoUser, WhoxFlags, WhoxPresence, WhoxReply,
};

impl State {
    fn apply_user_tags(&mut self, message: &MessageRef<'_>, outbox: &mut Outbox) {
        let Some(Source::User { nick, .. }) = message.source() else {
            return;
        };
        if let Some(user) = self.users.get_mut(&Folded::new(nick, self.casemapping)) {
            for (key, value) in message.tags() {
                match (key, value) {
                    ("account", Some(account)) => {
                        let account = Account::LoggedIn(account.as_ref().into());
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
            self.nick = own_nick.into();
        }
        self.apply_user_tags(message, outbox);
    }

    pub(super) fn prepare_message(
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
        if let Some(user) = self.users.get_mut(&Folded::new(nick, self.casemapping))
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

    fn handle_away(
        &mut self,
        message: &MessageRef<'_>,
        own_away_message: Option<&str>,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        if let Some(away) = AwayNotify::parse(message) {
            let Source::User { nick, .. } = message.source()? else {
                return Some(Handled::No);
            };
            if let Some(user) = self.users.get_mut(&Folded::new(nick, self.casemapping)) {
                let next = away.message.map_or(Away::Present, |message| Away::Away {
                    message: Some(message.to_owned()),
                });
                if user.away != next {
                    user.away = next;
                    outbox.event(Event::AwayChanged {
                        nick: nick.to_owned(),
                        message: away.message.map(str::to_owned),
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
                if let Some(user) = self.users.get_mut(&Folded::new(nick, self.casemapping)) {
                    let message = away.to_owned();
                    let next = Away::Away {
                        message: Some(message.clone()),
                    };
                    if user.away != next {
                        user.away = next;
                        outbox.event(Event::AwayChanged {
                            nick: nick.to_owned(),
                            message: Some(message),
                        });
                    }
                }
            }
            Command::Numeric(Numeric::RPL_UNAWAY) if self.away => {
                self.away = false;
                self.away_message = None;
                outbox.event(Event::AwayChanged {
                    nick: self.nick.as_ref().to_owned(),
                    message: None,
                });
            }
            Command::Numeric(Numeric::RPL_NOWAWAY) if !self.away => {
                self.away = true;
                if let Some(message) = own_away_message {
                    self.away_message = Some(message.to_owned());
                    outbox.event(Event::AwayChanged {
                        nick: self.nick.as_ref().to_owned(),
                        message: self.away_message.clone(),
                    });
                }
            }
            Command::Numeric(Numeric::RPL_UNAWAY | Numeric::RPL_NOWAWAY) => {}
            _ => return None,
        }
        Some(Handled::Yes)
    }

    pub(super) fn handle_user_update(
        &mut self,
        message: &MessageRef<'_>,
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
        message: &MessageRef<'_>,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        if message.command() != Command::Numeric(Numeric::RPL_VISIBLEHOST) {
            return None;
        }
        let (Some(nick), Some(host)) = (message.param(0), message.param(1)) else {
            return Some(Handled::No);
        };
        if !self.casemapping.eq(nick, &self.nick) {
            return Some(Handled::No);
        }
        self.host = Some(host.to_owned());
        outbox.event(Event::HostChanged {
            nick: self.nick.as_ref().to_owned(),
            user: self.user.clone(),
            host: host.to_owned(),
        });
        Some(Handled::Yes)
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
            if let Some(user) = self.users.get_mut(&Folded::new(nick, self.casemapping)) {
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
        if let Some(user) = self.users.get_mut(&Folded::new(nick, self.casemapping)) {
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
            let channel = Folded::new(message.param(1)?, self.casemapping);
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
                .contains(&Folded::new(channel, self.casemapping))
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
        if let Some(user) = self.users.get_mut(&Folded::new(nick, self.casemapping)) {
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
            .get_mut(&Folded::new(reply.nick, self.casemapping))
        {
            user.realname = reply.realname.map(str::to_owned);
            user.away = match flags.presence {
                Some(WhoxPresence::Gone) => Away::Away { message: None },
                Some(WhoxPresence::Here) => Away::Present,
                None => user.away.clone(),
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
            .contains(&Folded::new(channel, self.casemapping))
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
        let key = Folded::new(channel, self.casemapping);
        if !self.who_sweeps.insert(key) {
            return;
        }
        if isupport.whox() {
            outbox.transmit("WHO", [channel, WHOX_FIELDS]);
        } else {
            outbox.transmit("WHO", [channel]);
        }
    }
}
