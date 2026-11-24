use super::{
    AccountNotify, AccountStatus, AwayNotify, AwayStatus, Chghost, Command, Event, Handled,
    Isupport, LineRef, NameKey, Numeric, Outbox, Party, ProtoSource, Setname, State, User,
    WHOX_FIELDS, WHOX_TOKEN, WhoSweep, WhoUser, WhoxFlags, WhoxPresence, WhoxReply,
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

    /// The account a user's line attests. Under `account-tag`, "If the user is not identified to
    /// any services account, the tag MUST NOT be sent", so another user's line without it means
    /// logged out. Our own account follows the 900 and 901 numerics instead.
    fn line_account(
        &self,
        message: &LineRef<'_>,
        nick: &str,
        enabled_caps: &crate::Capabilities,
    ) -> Option<AccountStatus> {
        if message.command() == Command::Account {
            return None;
        }
        match message.tag("account") {
            Some(account) => Some(AccountStatus::LoggedIn(
                self.casemapping.account(account.as_ref()),
            )),
            None => (enabled_caps.contains_key(&crate::cap::ACCOUNT_TAG)
                && !crate::names_equal(self.casemapping, nick, self.nick.as_str()))
            .then_some(AccountStatus::LoggedOut),
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
        let line_account = if crate::LineEnvelope::command_account(message).is_some() {
            None
        } else {
            self.line_account(message, nick, enabled_caps)
        };
        let token = self.__reply_token_for(nick);
        let mut changed = false;
        if let Some(user) = self.users.get_mut(self.casemapping.fold(nick).as_ref()) {
            if let Some(account) = line_account
                && user.account != account
            {
                let learned_logged_out =
                    user.account == AccountStatus::Unknown && account == AccountStatus::LoggedOut;
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
        }
        if changed {
            self.mark_changed();
        }
    }

    pub(super) fn apply_welcome(&mut self, message: &LineRef<'_>, own_nick: &str) {
        if message.command() == Command::Numeric(Numeric::RPL_WELCOME) {
            let old_nick = self.nick.clone();
            self.rename_user(old_nick.as_str(), own_nick);
            self.nick = self.casemapping.nick(own_nick);
            self.welcome_time = crate::server_clock::line_server_time(message);
        }
    }

    pub(crate) fn observe_live_identity(
        &mut self,
        message: &LineRef<'_>,
        isupport: &Isupport,
        enabled_caps: &crate::Capabilities,
        who_target: Option<&str>,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        self.update_casemapping(isupport.casemapping());
        self.observe_user_source(message, isupport, enabled_caps);
        let account = self.handle_account(message, outbox);
        self.apply_user_tags(message, enabled_caps, outbox);
        let _ = self.observe_subject(message, isupport);
        account.or_else(|| self.handle_who(message, isupport, who_target, outbox))
    }

    /// Whether papo would see `user` leave or change nick: through a shared channel, MONITOR, or
    /// because it is us.
    fn follows_presence(&self, user: &User) -> bool {
        !user.channels.is_empty() || user.monitored || user.nick == self.nick
    }

    fn reconcile_message_user(
        &mut self,
        nick: &str,
        username: Option<&str>,
        host: Option<&str>,
        account: Option<&AccountStatus>,
    ) -> Option<&mut User> {
        let folded = self.casemapping.fold(nick);
        let user = self.users.get(folded.as_ref())?;
        let followed = self.follows_presence(user);
        if followed && !user.unconfirmed_presence {
            return None;
        }
        if !user.may_send_as(username, host, account) {
            let mut replacement = self.new_user(nick, username, host);
            if followed {
                let key = NameKey::new(nick, self.casemapping);
                let previous = self.users.remove(&key).expect("known user");
                self.prepare_identity_arrival(nick);
                self.remember_departed(&previous);
                replacement.channels = previous.channels;
                replacement.monitored = previous.monitored;
            } else {
                self.prepare_identity_arrival(nick);
            }
            replacement.unconfirmed_presence = true;
            if let Some(account) = account {
                replacement.update_account(account.clone());
            }
            self.users
                .insert(NameKey::new(nick, self.casemapping), replacement);
            self.mark_changed();
        } else if (user.username.is_none() && username.is_some())
            || (user.host.is_none() && host.is_some())
        {
            self.users
                .get_mut(folded.as_ref())?
                .update_userhost(username, host);
            self.mark_changed();
        }
        let user = self.users.get_mut(folded.as_ref())?;
        if followed {
            user.confirm_presence(username, host);
        }
        Some(user)
    }

    fn reconcile_account_source(&mut self, message: &LineRef<'_>, account: &AccountStatus) {
        let Some(ProtoSource::User {
            nick,
            username,
            host,
            ..
        }) = message.source()
        else {
            return;
        };
        if let Some(user) = self.reconcile_message_user(nick, username, host, Some(account))
            && message.tags().any(|(key, _)| key == "bot")
        {
            user.is_bot = Some(true);
        }
    }

    fn observe_user_source(
        &mut self,
        message: &LineRef<'_>,
        isupport: &Isupport,
        enabled_caps: &crate::Capabilities,
    ) {
        let Some(ProtoSource::User {
            nick,
            username,
            host,
            ..
        }) = message.source()
        else {
            return;
        };
        if crate::LineEnvelope::command_account(message).is_some() {
            if message.command() == Command::Join {
                let account =
                    crate::LineEnvelope::__from_line_with_account(message, self.casemapping.into())
                        .__reported_account(self.casemapping.into(), self.casemapping.origin())
                        .expect("extended JOIN has command evidence");
                self.reconcile_account_source(message, &account);
            }
            return;
        }
        let line_account = self.line_account(message, nick, enabled_caps);
        self.reconcile_message_user(nick, username, host, line_account.as_ref());
        if let Some(change) = Chghost::parse(message) {
            self.reconcile_message_user(
                nick,
                Some(change.user),
                Some(change.host),
                line_account.as_ref(),
            );
        }
        if !matches!(
            message.command(),
            Command::Privmsg | Command::Notice | Command::Tagmsg | Command::Redact
        ) {
            return;
        }
        let key = NameKey::new(nick, self.casemapping);
        let mut changed = false;
        if !self.users.contains_key(&key) {
            let mut user = self.new_user(nick, username, host);
            user.unconfirmed_presence = true;
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
        AccountNotify::parse(message)?;
        let Some(ProtoSource::User { nick, .. }) = message.source() else {
            return Some(Handled::No);
        };
        let envelope =
            crate::LineEnvelope::__from_line_with_account(message, self.casemapping.into());
        let account = envelope
            .__reported_account(self.casemapping.into(), self.casemapping.origin())
            .expect("ACCOUNT has command evidence");
        let actor = message.raw_source().unwrap_or(nick).to_owned();
        let changed_or_untracked = self.user(nick).is_none_or(|user| user.account != account);
        let previous_token = self.__reply_token_for(nick);
        self.reconcile_account_source(message, &account);
        let token = self.__reply_token_for(nick);
        if changed_or_untracked || token != previous_token {
            if let Some(user) = self.users.get_mut(self.casemapping.fold(nick).as_ref()) {
                user.update_account(account.clone());
            }
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
                None => AwayStatus::Present,
                Some("*") => AwayStatus::Away { message: None },
                Some(message) => AwayStatus::Away {
                    message: Some(message.to_owned()),
                },
            };
            let token = self.__reply_token_for(nick);
            if self.update_away(nick, &next) {
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
                let next = AwayStatus::Away {
                    message: Some(away.to_owned()),
                };
                if self.update_away(nick, &next) {
                    outbox.event(Event::AwayChanged {
                        envelope: crate::LineEnvelope::from_line(message),
                        actor: nick.to_owned(),
                        nick: self.casemapping.nick(nick),
                        away: next,
                        token,
                        actor_party: self.nick_party(nick),
                    });
                }
            }
            Command::Numeric(Numeric::RPL_UNAWAY)
                if self.own_user().away != AwayStatus::Present =>
            {
                self.users
                    .get_mut(self.nick.key())
                    .expect("own user is always present")
                    .away = AwayStatus::Present;
                outbox.event(Event::AwayChanged {
                    envelope: crate::LineEnvelope::from_line(message),
                    actor: self.nick.to_string(),
                    nick: self.nick.clone(),
                    away: AwayStatus::Present,
                    token: self.__reply_token_for(self.nick.as_str()),
                    actor_party: Party::Me,
                });
            }
            Command::Numeric(Numeric::RPL_NOWAWAY)
                if !matches!(self.own_user().away, AwayStatus::Away { .. }) =>
            {
                let away = AwayStatus::Away {
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

    fn update_away(&mut self, nick: &str, next: &AwayStatus) -> bool {
        self.users
            .get_mut(self.casemapping.fold(nick).as_ref())
            .is_none_or(|user| {
                if user.away == *next {
                    return false;
                }
                user.away = next.clone();
                true
            })
    }

    pub(super) fn handle_user_update(
        &mut self,
        message: &LineRef<'_>,
        own_away_message: Option<&str>,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        self.handle_visible_host(message, outbox)
            .or_else(|| self.handle_away(message, own_away_message, outbox))
            .or_else(|| self.handle_identity_change(message, outbox))
            .or_else(|| self.handle_bot_numeric(message))
    }

    fn observe_subject(&mut self, message: &LineRef<'_>, isupport: &Isupport) -> Option<()> {
        if let Some((_, _, entries)) = super::parse_353(message) {
            for raw in entries {
                let entry = super::NamesEntry::parse(raw, isupport);
                self.reconcile_message_user(entry.nick, entry.user, entry.host, None);
            }
            return Some(());
        }
        let nick = message.param(1)?;
        match message.command() {
            Command::Numeric(Numeric::RPL_MONONLINE) => {
                for (nick, username, host) in
                    nick.split(',').filter_map(crate::monitor::parse_entry)
                {
                    self.reconcile_message_user(nick, username, host, None);
                }
            }
            Command::Numeric(Numeric::RPL_WHOISUSER) => {
                self.reconcile_message_user(
                    nick,
                    Some(message.param(2)?),
                    Some(message.param(3)?),
                    None,
                );
            }
            Command::Numeric(Numeric::RPL_WHOISACCOUNT) => {
                let account = AccountStatus::from_notify(Some(message.param(2)?), self.casemapping);
                self.observe_who_facts(nick, None, None, Some(&account));
            }
            Command::Numeric(Numeric::RPL_USERHOST) => {
                for entry in nick.split_whitespace() {
                    if let Some((nick, userhost)) = entry.split_once('=')
                        && let Some(userhost) = userhost.strip_prefix(['+', '-'])
                    {
                        let (username, host) = userhost
                            .split_once('@')
                            .map_or((None, userhost), |(username, host)| (Some(username), host));
                        let nick = nick.strip_suffix('*').unwrap_or(nick);
                        self.reconcile_message_user(
                            nick,
                            username.filter(|user| !user.is_empty()),
                            Some(host).filter(|host| !host.is_empty()),
                            None,
                        );
                    }
                }
            }
            _ => return None,
        }
        Some(())
    }

    fn observe_who_facts(
        &mut self,
        nick: &str,
        username: Option<&str>,
        host: Option<&str>,
        account: Option<&AccountStatus>,
    ) {
        let user = self.reconcile_message_user(nick, username, host, account);
        if let (Some(user), Some(account)) = (user, account)
            && user.account != *account
        {
            user.update_account(account.clone());
            self.mark_changed();
        }
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
            self.mark_changed();
        }
        Some(Handled::No)
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
                let full = sweep.realname_changes.len() >= self.max_members_per_channel
                    && !sweep.realname_changes.contains(&nick_key);
                if sweep.track_overlay(&nick_key, full) {
                    sweep.realname_changes.insert(nick_key.clone());
                }
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
        who_target: Option<&str>,
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
            return self.handle_whox_reply(message, &reply, isupport, who_target, outbox);
        }
        self.handle_plain_who(message, isupport, who_target, outbox)
    }

    fn handle_whox_reply(
        &mut self,
        message: &LineRef<'_>,
        reply: &WhoxReply<'_>,
        isupport: &Isupport,
        who_target: Option<&str>,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        let nick = reply.nick?;
        if reply.token != Some(WHOX_TOKEN) {
            return None;
        }
        let flags = reply.flags(isupport)?;
        self.apply_who_user(
            message,
            WhoUser {
                channel: reply.channel.unwrap_or("*"),
                nick,
                username: reply.user,
                host: reply.host,
                account: Some(AccountStatus::from_notify(reply.account, self.casemapping)),
                realname: reply.realname,
            },
            &flags,
            who_target,
            outbox,
        )
    }

    fn apply_who_user(
        &mut self,
        message: &LineRef<'_>,
        reply: WhoUser<'_>,
        flags: &WhoxFlags,
        who_target: Option<&str>,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        let channel = self.casemapping.channel(reply.channel);
        let nick = NameKey::new(reply.nick.to_owned(), self.casemapping);
        let sweep = self.who_sweeps.get(&channel);
        if sweep.is_some_and(|sweep| sweep.row_is_stale(&nick)) {
            return Some(Handled::Yes);
        }
        let tracked_channel = sweep.is_some();
        let prefix_changes = sweep
            .and_then(|sweep| sweep.prefix_changes.get(&nick))
            .cloned();
        let realname_changed = sweep.is_some_and(|sweep| sweep.realname_changes.contains(&nick));
        self.observe_who_facts(
            reply.nick,
            reply.username,
            reply.host,
            reply.account.as_ref(),
        );
        if !tracked_channel {
            return self.apply_nick_who_away(
                message,
                who_target,
                reply.nick,
                flags.presence,
                outbox,
            );
        }
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
            user.is_bot = Some(flags.bot);
        }
        self.apply_who_away(message, reply.nick, flags.presence, outbox);
        if let Some(account) = reply.account
            && let Some(user) = self
                .users
                .get_mut(self.casemapping.fold(reply.nick).as_ref())
        {
            user.update_account(account);
        }
        Some(Handled::Yes)
    }

    fn apply_who_away(
        &mut self,
        message: &LineRef<'_>,
        nick: &str,
        presence: Option<WhoxPresence>,
        outbox: &mut Outbox,
    ) {
        let Some(user) = self.users.get_mut(self.casemapping.fold(nick).as_ref()) else {
            return;
        };
        let away = match (presence, &user.away) {
            (Some(WhoxPresence::Here), AwayStatus::Unknown | AwayStatus::Away { .. }) => {
                AwayStatus::Present
            }
            (Some(WhoxPresence::Gone), AwayStatus::Unknown | AwayStatus::Present) => {
                AwayStatus::Away { message: None }
            }
            _ => return,
        };
        let known = user.away != AwayStatus::Unknown;
        user.away = away.clone();
        if known {
            outbox.event(Event::AwayChanged {
                envelope: crate::LineEnvelope::from_line(message),
                actor: nick.to_owned(),
                nick: self.casemapping.nick(nick),
                away,
                token: self.__reply_token_for(nick),
                actor_party: self.nick_party(nick),
            });
        }
    }

    fn handle_plain_who(
        &mut self,
        message: &LineRef<'_>,
        isupport: &Isupport,
        who_target: Option<&str>,
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
        let realname = hopcount_realname.split_once(' ')?.1;
        let flags = WhoxFlags::__parse(raw_flags, isupport);
        self.apply_who_user(
            message,
            WhoUser {
                channel,
                nick,
                username: Some(user),
                host: Some(host),
                account: None,
                realname: Some(realname),
            },
            &flags,
            who_target,
            outbox,
        )
    }

    fn apply_nick_who_away(
        &mut self,
        message: &LineRef<'_>,
        target: Option<&str>,
        nick: &str,
        presence: Option<WhoxPresence>,
        outbox: &mut Outbox,
    ) -> Option<Handled> {
        if !self.names_equal(target?, nick) {
            return None;
        }
        self.apply_who_away(message, nick, presence, outbox);
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
    pub(super) fn confirm_presence(&mut self, username: Option<&str>, host: Option<&str>) {
        // A bare roster entry cannot connect the current holder to an earlier message.
        if username.is_some() && host.is_some() {
            self.unconfirmed_presence = false;
        }
    }

    /// Whether a line from this user's nick, sent from `username@host` with the line's `account`,
    /// can come from the person papo knows. A person papo cannot follow may have left unseen and
    /// another may hold the nick, so any changed fact means a new person.
    fn may_send_as(
        &self,
        username: Option<&str>,
        host: Option<&str>,
        account: Option<&AccountStatus>,
    ) -> bool {
        let unchanged = |known: Option<&str>, seen: Option<&str>| {
            known.zip(seen).is_none_or(|(known, seen)| known == seen)
        };
        unchanged(self.username.as_deref(), username)
            && unchanged(self.host.as_deref(), host)
            && account.is_none_or(|account| {
                self.account == AccountStatus::Unknown || self.account == *account
            })
    }
}
