use super::{
    Account, AccountNotify, Away, AwayNotify, Casemapping, Chghost, Command, Event, Handled,
    Isupport, LineRef, Numeric, Outbox, Setname, Source, State, WHOX_FIELDS, WHOX_TOKEN, WhoUser,
    WhoxFlags, WhoxPresence, WhoxReply,
};

impl State {
    fn apply_user_tags(&mut self, message: &LineRef<'_>, outbox: &mut Outbox) {
        let Some(Source::User { nick, .. }) = message.source() else {
            return;
        };
        if let Some(user) = self.users.get_mut(&self.casemapping.nick(nick)) {
            for (key, value) in message.tags() {
                match (key, value) {
                    ("account", Some(account)) => {
                        let account = Account::LoggedIn(self.casemapping.account(account.as_ref()));
                        if user.account != account {
                            user.account = account.clone();
                            outbox.event(Event::AccountChanged {
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
    ) -> Casemapping {
        let mapping = isupport.casemapping();
        self.update_casemapping(mapping);
        self.apply_message_metadata(message, own_nick, outbox);
        mapping
    }

    fn handle_account(&mut self, message: &LineRef<'_>, outbox: &mut Outbox) -> Option<Handled> {
        let account = AccountNotify::parse(message)?;
        let Some(Source::User { nick, .. }) = message.source() else {
            return Some(Handled::No);
        };
        let account = Account::from_notify(account.account, self.casemapping);
        if let Some(user) = self.users.get_mut(&self.casemapping.nick(nick))
            && user.account != account
        {
            user.account = account.clone();
            outbox.event(Event::AccountChanged {
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
            let Source::User { nick, .. } = message.source()? else {
                return Some(Handled::No);
            };
            if let Some(user) = self.users.get_mut(&self.casemapping.nick(nick)) {
                let next = match away.message {
                    None => Away::Present,
                    Some("*") => Away::Away { message: None },
                    Some(message) => Away::Away {
                        message: Some(message.to_owned()),
                    },
                };
                if user.away != next {
                    user.away = next;
                    outbox.event(Event::AwayChanged {
                        nick: self.casemapping.nick(nick),
                        message: away
                            .message
                            .filter(|message| *message != "*")
                            .map(str::to_owned),
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
                if let Some(user) = self.users.get_mut(&self.casemapping.nick(nick)) {
                    let message = away.to_owned();
                    let next = Away::Away {
                        message: Some(message.clone()),
                    };
                    if user.away != next {
                        user.away = next;
                        outbox.event(Event::AwayChanged {
                            nick: self.casemapping.nick(nick),
                            message: Some(message),
                        });
                    }
                }
            }
            Command::Numeric(Numeric::RPL_UNAWAY) if self.away => {
                self.away = false;
                self.away_message = None;
                outbox.event(Event::AwayChanged {
                    nick: self.nick.clone(),
                    message: None,
                });
            }
            Command::Numeric(Numeric::RPL_NOWAWAY) if !self.away => {
                self.away = true;
                if let Some(message) = own_away_message {
                    self.away_message = Some(message.to_owned());
                    outbox.event(Event::AwayChanged {
                        nick: self.nick.clone(),
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
            nick: self.nick.clone(),
            user: self.user.clone(),
            host: host.to_owned(),
        });
        Some(Handled::Yes)
    }

    fn handle_identity_change(
        &mut self,
        message: &LineRef<'_>,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        let Source::User { nick, .. } = message.source()? else {
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
                nick: self.casemapping.nick(nick),
                user: change.user.to_owned(),
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
            || !self.who_sweeps.contains(&self.casemapping.channel(channel))
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
            user.away = match (&user.away, flags.presence) {
                (Away::Present, Some(WhoxPresence::Gone)) => Away::Away { message: None },
                (_, Some(WhoxPresence::Here)) => Away::Present,
                (_, Some(WhoxPresence::Gone) | None) => user.away.clone(),
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
        if !self.who_sweeps.contains(&self.casemapping.channel(channel)) {
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
