use super::{
    Account, AccountNotify, Away, AwayNotify, Chghost, Command, Event, Handled, Isupport, LineRef,
    NameKey, Numeric, Outbox, Party, ProtoSource, Setname, State, User, WHOX_FIELDS, WHOX_TOKEN,
    WhoSweep, WhoUser, WhoxFlags, WhoxPresence, WhoxReply,
};

impl State {
    /// Returns the identity of the current holder of the nickname a `RPL_WHOISUSER`,
    /// `RPL_WHOREPLY` or WHOX row describes.
    pub(crate) fn who_subject_token(&self, message: &LineRef<'_>) -> Option<crate::ReplyToken> {
        let nick = match message.command() {
            Command::Numeric(Numeric::RPL_WHOISUSER) => message.param(1),
            Command::Numeric(Numeric::RPL_WHOREPLY) => message.param(5),
            _ => WhoxReply::parse(message, WHOX_FIELDS).and_then(|reply| reply.nick),
        }?;
        Some(self.__reply_token_for(nick))
    }

    /// Returns the account a WHOX `%na` reply lists for `nick`, matching the row's nickname under
    /// the session's casemapping, or `None` when no row names `nick`.
    #[doc(hidden)]
    #[must_use]
    pub fn __who_account(&self, nick: &str, lines: &[papo_proto::Line]) -> Option<Account> {
        let account = lines.iter().find_map(|line| {
            let line = line.as_ref();
            (line.command_is("354")
                && line
                    .param(1)
                    .is_some_and(|found| self.names_equal(found, nick)))
            .then(|| line.param(2))
            .flatten()
        })?;
        Some(match account {
            "0" | "*" => Account::LoggedOut,
            account => Account::LoggedIn(self.casemapping.account(account)),
        })
    }

    /// The account a user's line attests. Under `account-tag`, "If the user is not identified to
    /// any services account, the tag MUST NOT be sent", so another user's line without it means
    /// logged out. Our own account follows the 900 and 901 numerics instead.
    fn line_account(
        &self,
        message: &LineRef<'_>,
        nick: &str,
        enabled_caps: &crate::Capabilities,
    ) -> Option<Account> {
        match message.tag("account") {
            Some(account) => Some(Account::LoggedIn(
                self.casemapping.account(account.as_ref()),
            )),
            None => (enabled_caps.contains_key(&crate::cap::ACCOUNT_TAG)
                && !crate::names_equal(self.casemapping, nick, self.nick.as_str()))
            .then_some(Account::LoggedOut),
        }
    }

    fn apply_user_tags(
        &mut self,
        message: &LineRef<'_>,
        enabled_caps: &crate::Capabilities,
        outbox: &mut Outbox,
    ) {
        let Some(ProtoSource::User { nick, .. }) = message.source() else {
            return;
        };
        let actor = message.raw_source().unwrap_or(nick).to_owned();
        let actor_party = self.source_party(message);
        let line_account = self.line_account(message, nick, enabled_caps);
        // NONSTANDARD(inspircd): InspIRCd sends a never-reused account ID in this vendor tag next to
        // `account`; it has no cap of its own, and without the tag pins fall back to the account name.
        let account_id = message
            .tag("account")
            .zip(message.tag("inspircd.org/account-id"))
            .map(|(_, id)| id.into_owned())
            .filter(|id| !id.is_empty());
        let token = self.__reply_token_for(nick);
        let mut changed = false;
        if let Some(user) = self.users.get_mut(self.casemapping.fold(nick).as_ref()) {
            if let Some(account) = line_account
                && user.account != account
            {
                let learned_logged_out =
                    user.account == Account::Unknown && account == Account::LoggedOut;
                user.update_account(account.clone());
                changed = true;
                if !learned_logged_out {
                    outbox.event(Event::AccountChanged {
                        envelope: crate::LineEnvelope::from_line(message),
                        actor,
                        nick: self.casemapping.nick(nick),
                        account,
                        token,
                        actor_party,
                    });
                }
            }
            if user.is_bot != Some(true) && message.tags().any(|(key, _)| key == "bot") {
                user.is_bot = Some(true);
                changed = true;
            }
            if let Some(account_id) = account_id
                && user.account_id.as_deref() != Some(account_id.as_str())
            {
                user.account_id = Some(account_id);
                changed = true;
            }
        }
        if changed {
            self.mark_changed();
        }
    }

    fn apply_message_metadata(
        &mut self,
        message: &LineRef<'_>,
        own_nick: &str,
        enabled_caps: &crate::Capabilities,
        outbox: &mut Outbox,
    ) {
        if message.command() == Command::Numeric(Numeric::RPL_WELCOME) {
            let old_nick = self.nick.clone();
            self.rename_user(old_nick.as_str(), own_nick);
            self.nick = self.casemapping.nick(own_nick);
            self.welcome_time = crate::server_clock::line_server_time(message);
        }
        self.apply_user_tags(message, enabled_caps, outbox);
    }

    pub(super) fn prepare_message(
        &mut self,
        message: &LineRef<'_>,
        own_nick: &str,
        isupport: &Isupport,
        enabled_caps: &crate::Capabilities,
        outbox: &mut Outbox,
    ) -> crate::Names {
        let mapping = isupport.casemapping();
        self.update_casemapping(mapping);
        self.track_message_user(message, isupport, enabled_caps);
        self.apply_message_metadata(message, own_nick, enabled_caps, outbox);
        self.casemapping
    }

    /// Whether papo would see `user` leave or change nick: through a shared channel, MONITOR, or
    /// because it is us.
    fn follows_presence(&self, user: &User) -> bool {
        !user.channels.is_empty() || user.monitored || user.nick == self.nick
    }

    fn track_message_user(
        &mut self,
        message: &LineRef<'_>,
        isupport: &Isupport,
        enabled_caps: &crate::Capabilities,
    ) {
        if !matches!(
            message.command(),
            Command::Privmsg | Command::Notice | Command::Tagmsg | Command::Redact
        ) {
            return;
        }
        let Some(ProtoSource::User {
            nick,
            username,
            host,
            ..
        }) = message.source()
        else {
            return;
        };
        let key = NameKey::new(nick, self.casemapping);
        let line_account = self.line_account(message, nick, enabled_caps);
        if self.users.get(&key).is_some_and(|user| {
            !self.follows_presence(user) && !user.may_send_as(username, host, line_account.as_ref())
        }) {
            self.prepare_identity_arrival(nick);
        }
        let mut changed = false;
        if !self.users.contains_key(&key) {
            let mut user = match self.take_proven(nick, line_account.as_ref()) {
                Some((identity, account)) => {
                    let mut user = self.user_with_identity(nick, username, host, identity);
                    user.proven_account = account;
                    user
                }
                None => self.new_user(nick, username, host),
            };
            if let Some(account) = line_account {
                user.update_account(account);
            }
            self.users.insert(key.clone(), user);
            changed = true;
        }
        let message_tags = enabled_caps.contains_key(&crate::cap::MESSAGE_TAGS);
        if let Some(user) = self.users.get_mut(&key) {
            let old_username = user.username.clone();
            let old_host = user.host.clone();
            let old_is_bot = user.is_bot;
            user.update_userhost(username, host);
            if message_tags && isupport.bot_mode().is_some() {
                user.is_bot = Some(message.tags().any(|(name, _)| name == "bot"));
            }
            changed |=
                old_username != user.username || old_host != user.host || old_is_bot != user.is_bot;
        }
        self.message_users.retain(|known| known != &key);
        self.message_users.push_back(key);
        if self.message_users.len() > super::MAX_MESSAGE_USERS
            && let Some(evicted) = self.message_users.pop_front()
            && self.users.get(&evicted).is_some_and(|user| {
                user.nick != self.nick && !user.monitored && user.channels.is_empty()
            })
        {
            changed |= self.users.remove(&evicted).is_some();
        }
        if changed {
            self.mark_changed();
        }
    }

    fn handle_account(&mut self, message: &LineRef<'_>, outbox: &mut Outbox) -> Option<Handled> {
        let account = AccountNotify::parse(message)?;
        let Some(ProtoSource::User { nick, .. }) = message.source() else {
            return Some(Handled::No);
        };
        let envelope = crate::LineEnvelope::from_line(message).with_account(account.account);
        let account = Account::from_notify(account.account, self.casemapping);
        let actor = message.raw_source().unwrap_or(nick).to_owned();
        let token = self.__reply_token_for(nick);
        if let Some(user) = self.users.get_mut(self.casemapping.fold(nick).as_ref())
            && user.account != account
        {
            user.update_account(account.clone());
            outbox.event(Event::AccountChanged {
                envelope,
                actor,
                nick: self.casemapping.nick(nick),
                account,
                token,
                actor_party: self.source_party(message),
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
            let token = self.__reply_token_for(nick);
            let user_changed = self
                .users
                .get_mut(self.casemapping.fold(nick).as_ref())
                .is_some_and(|user| {
                    if user.away == next {
                        false
                    } else {
                        user.away = next.clone();
                        true
                    }
                });
            if user_changed {
                outbox.event(Event::AwayChanged {
                    envelope: crate::LineEnvelope::from_line(message),
                    actor: message.raw_source().unwrap_or(nick).to_owned(),
                    nick: self.casemapping.nick(nick),
                    away: next,
                    token,
                    actor_party: self.source_party(message),
                });
            }
            return Some(Handled::Yes);
        }
        match message.command() {
            Command::Numeric(Numeric::RPL_AWAY) => {
                let (Some(nick), Some(away)) = (message.param(1), message.param(2)) else {
                    return Some(Handled::No);
                };
                let token = self.__reply_token_for(nick);
                if let Some(user) = self.users.get_mut(self.casemapping.fold(nick).as_ref()) {
                    let envelope = crate::LineEnvelope::from_line(message);
                    let message = away.to_owned();
                    let next = Away::Away {
                        message: Some(message.clone()),
                    };
                    if user.away != next {
                        user.away = next.clone();
                        outbox.event(Event::AwayChanged {
                            envelope,
                            actor: nick.to_owned(),
                            nick: self.casemapping.nick(nick),
                            away: next,
                            token,
                            actor_party: self.nick_party(nick),
                        });
                    }
                }
            }
            Command::Numeric(Numeric::RPL_UNAWAY) if self.own_user().away != Away::Present => {
                self.users
                    .get_mut(self.nick.key())
                    .expect("own user is always present")
                    .away = Away::Present;
                outbox.event(Event::AwayChanged {
                    envelope: crate::LineEnvelope::from_line(message),
                    actor: self.nick.to_string(),
                    nick: self.nick.clone(),
                    away: Away::Present,
                    token: self.__reply_token_for(self.nick.as_str()),
                    actor_party: Party::Me,
                });
            }
            Command::Numeric(Numeric::RPL_NOWAWAY)
                if !matches!(self.own_user().away, Away::Away { .. }) =>
            {
                let away = Away::Away {
                    message: own_away_message.map(str::to_owned),
                };
                self.users
                    .get_mut(self.nick.key())
                    .expect("own user is always present")
                    .away = away.clone();
                outbox.event(Event::AwayChanged {
                    envelope: crate::LineEnvelope::from_line(message),
                    actor: self.nick.to_string(),
                    nick: self.nick.clone(),
                    away,
                    token: self.__reply_token_for(self.nick.as_str()),
                    actor_party: Party::Me,
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
            .or_else(|| self.handle_bot_numeric(message))
            .or_else(|| self.handle_who(message, isupport, outbox))
    }

    fn handle_bot_numeric(&mut self, message: &LineRef<'_>) -> Option<Handled> {
        if message.command() != Command::Numeric(Numeric::RPL_WHOISBOT) {
            return None;
        }
        let Some(nick) = message.param(1) else {
            return Some(Handled::No);
        };
        if let Some(user) = self.users.get_mut(self.casemapping.fold(nick).as_ref()) {
            user.is_bot = Some(true);
        }
        Some(Handled::Yes)
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
        if !crate::names_equal(self.casemapping, nick, self.nick.as_str()) {
            return Some(Handled::No);
        }
        let fallback_username = self.nick.to_string();
        let token = self.__reply_token_for(self.nick.as_str());
        let Some(own) = self.users.get_mut(self.nick.key()) else {
            return Some(Handled::No);
        };
        own.host = Some(host.to_owned());
        outbox.event(Event::HostChanged {
            envelope: crate::LineEnvelope::from_line(message),
            actor: self.nick.to_string(),
            nick: self.nick.clone(),
            username: own.username.clone().unwrap_or(fallback_username),
            host: host.to_owned(),
            token,
            actor_party: Party::Me,
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
        let token = self.__reply_token_for(nick);
        if let Some(change) = Chghost::parse(message) {
            if let Some(user) = self.users.get_mut(self.casemapping.fold(nick).as_ref()) {
                user.update_userhost(Some(change.user), Some(change.host));
            }
            outbox.event(Event::HostChanged {
                envelope: crate::LineEnvelope::from_line(message),
                actor: message.raw_source().unwrap_or(nick).to_owned(),
                nick: self.casemapping.nick(nick),
                username: change.user.to_owned(),
                host: change.host.to_owned(),
                token,
                actor_party: self.source_party(message),
            });
            return Some(Handled::Yes);
        }
        let change = Setname::parse(message)?;
        let mut channels = Vec::new();
        if let Some(user) = self.users.get_mut(self.casemapping.fold(nick).as_ref()) {
            user.realname = Some(change.realname.to_owned());
            channels.extend(user.channels.iter().cloned());
        }
        let nick_key = NameKey::new(nick.to_owned(), self.casemapping);
        for channel in channels {
            if let Some(sweep) = self.who_sweeps.get_mut(&channel) {
                sweep.realname_changes.insert(nick_key.clone());
            }
        }
        let envelope = crate::LineEnvelope::from_line(message);
        outbox.event(Event::RealnameChanged {
            server_time: envelope.server_time(),
            envelope,
            actor: message.raw_source().unwrap_or(nick).to_owned(),
            nick: self.casemapping.nick(nick),
            realname: change.realname.to_owned(),
            token,
            actor_party: self.source_party(message),
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
            let sweep = self.who_sweeps.get_mut(&channel)?;
            sweep.count -= 1;
            if sweep.count == 0 {
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
        let applied = self.apply_who_user(
            WhoUser {
                channel,
                nick,
                username: reply.user,
                host: reply.host,
                realname: reply.realname,
            },
            &flags,
            outbox,
        );
        if applied && let Some(user) = self.users.get_mut(self.casemapping.fold(nick).as_ref()) {
            user.update_account(Account::from_notify(reply.account, self.casemapping));
        }
        Some(Handled::Yes)
    }

    fn apply_who_user(
        &mut self,
        reply: WhoUser<'_>,
        flags: &WhoxFlags,
        outbox: &mut Outbox,
    ) -> bool {
        let channel = self.casemapping.channel(reply.channel);
        let nick = NameKey::new(reply.nick.to_owned(), self.casemapping);
        let sweep = self.who_sweeps.get(&channel);
        if sweep.is_some_and(|sweep| sweep.row_is_stale(&nick)) {
            return false;
        }
        let prefix_changes = sweep
            .and_then(|sweep| sweep.prefix_changes.get(&nick))
            .cloned();
        let realname_changed = sweep.is_some_and(|sweep| sweep.realname_changes.contains(&nick));
        self.add_member(
            reply.channel,
            reply.nick,
            reply.username,
            reply.host,
            &flags.prefixes,
            outbox,
        );
        // Apply the snapshot first, then restore live changes observed while it was in flight.
        if let Some(prefix_changes) = prefix_changes
            && let Some(channel) = self.channel_mut(reply.channel)
        {
            for (prefix, action) in prefix_changes {
                channel.apply_prefix_change(reply.nick, prefix, action);
            }
        }
        if let Some(user) = self
            .users
            .get_mut(self.casemapping.fold(reply.nick).as_ref())
        {
            if !realname_changed {
                user.realname = reply.realname.map(str::to_owned);
            }
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
            user.is_bot = Some(flags.bot);
        }
        true
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
                .filter_map(|prefix| {
                    raw_flags
                        .contains(prefix.symbol())
                        .then_some(prefix.symbol())
                })
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
            &flags,
            outbox,
        );
        Some(Handled::Yes)
    }

    /// Marks `nick` as changed in the sweep of `channel`, or of every channel when `None`.
    pub(super) fn sweep_member_changed(&mut self, channel: Option<&str>, nick: &str) {
        let nick = NameKey::new(nick.to_owned(), self.casemapping);
        let limit = self.max_members_per_channel;
        match channel {
            Some(channel) => {
                if let Some(sweep) = self.who_sweeps.get_mut(&self.casemapping.channel(channel)) {
                    sweep.member_changed(nick, limit);
                }
            }
            None => {
                for sweep in self.who_sweeps.values_mut() {
                    sweep.member_changed(nick.clone(), limit);
                }
            }
        }
    }

    pub(super) fn start_who(&mut self, channel: &str, isupport: &Isupport, outbox: &mut Outbox) {
        let key = self.casemapping.channel(channel);
        if self.who_sweeps.contains_key(&key) {
            return;
        }
        self.who_sweeps.insert(key, WhoSweep::new());
        if isupport.whox() {
            outbox.transmit("WHO", [channel, WHOX_FIELDS]);
        } else {
            outbox.transmit("WHO", [channel]);
        }
    }

    pub(crate) fn track_who(&mut self, channel: &str) {
        let key = self.casemapping.channel(channel);
        if self
            .channels
            .contains_key(self.casemapping.fold(channel).as_ref())
        {
            self.who_sweeps
                .entry(key)
                .and_modify(|sweep| sweep.count += 1)
                .or_insert_with(WhoSweep::new);
        }
    }
}

impl User {
    /// Whether a line from this user's nick, sent from `username@host` with the line's `account`,
    /// can come from the person papo knows. A person papo cannot follow may have left unseen and
    /// another may hold the nick, so any changed fact means a new person.
    fn may_send_as(
        &self,
        username: Option<&str>,
        host: Option<&str>,
        account: Option<&Account>,
    ) -> bool {
        let unchanged = |known: Option<&str>, seen: Option<&str>| {
            known.zip(seen).is_none_or(|(known, seen)| known == seen)
        };
        unchanged(self.username.as_deref(), username)
            && unchanged(self.host.as_deref(), host)
            && account
                .is_none_or(|account| self.account == Account::Unknown || self.account == *account)
    }
}
