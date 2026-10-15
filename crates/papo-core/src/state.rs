use std::collections::HashMap;

use papo_proto::{CaseKey, Casemapping, Isupport, Join, MessageRef, Source};

use crate::{Config, Handled};

/// The known channel and user state for a session.
#[derive(Debug)]
pub struct State {
    nick: String,
    user: String,
    host: Option<String>,
    casemapping: Casemapping,
    channels: HashMap<CaseKey, Channel>,
}

/// State known about a joined channel.
#[derive(Debug)]
pub struct Channel {
    name: String,
    casemapping: Casemapping,
    members: HashMap<CaseKey, Member>,
    topic: Option<String>,
    synced: bool,
}

/// State known about a channel member.
#[derive(Debug)]
pub struct Member {
    nick: String,
    prefixes: Vec<char>,
}

impl State {
    pub(crate) fn new(config: &Config) -> Self {
        Self {
            nick: config.nick.clone(),
            user: config.username.clone(),
            host: None,
            casemapping: Casemapping::default(),
            channels: HashMap::new(),
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
        let Some(join) = Join::parse(message) else {
            return Handled::No;
        };
        let Some(Source::User { nick, user, host }) = message.source() else {
            return Handled::No;
        };
        let mapping = isupport.casemapping();
        self.casemapping = mapping;
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
                Channel::new(join.channel, mapping),
            );
        }
        if let Some(channel) = self.channels.get_mut(&CaseKey::new(join.channel, mapping)) {
            channel.members.insert(
                CaseKey::new(nick, mapping),
                Member {
                    nick: nick.to_owned(),
                    prefixes: Vec::new(),
                },
            );
        }
        Handled::Yes
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
}

impl Channel {
    fn new(name: &str, casemapping: Casemapping) -> Self {
        Self {
            name: name.to_owned(),
            casemapping,
            members: HashMap::new(),
            topic: None,
            synced: false,
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
}
