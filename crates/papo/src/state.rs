use papo_core::{Capabilities, Channel, Session, User};
use papo_proto::Isupport;

/// An immutable snapshot of an IRC client's known state.
#[derive(Clone, Debug)]
pub struct State {
    core: papo_core::State,
    #[allow(dead_code)]
    pub(crate) caps: Capabilities,
    #[allow(dead_code)]
    pub(crate) isupport: Isupport,
    account: Option<String>,
    connected: bool,
    generation: u64,
}

impl State {
    pub(crate) fn from_session(session: &Session, connected: bool) -> Self {
        Self {
            core: session.state().clone(),
            caps: session.caps().clone(),
            isupport: session.isupport().clone(),
            account: session.account().map(str::to_owned),
            connected,
            generation: session.state_generation(),
        }
    }

    pub(crate) const fn matches(&self, session: &Session, connected: bool) -> bool {
        self.generation == session.state_generation() && self.connected == connected
    }

    /// Reports whether the client currently has an open transport.
    #[must_use]
    pub const fn connected(&self) -> bool {
        self.connected
    }

    /// Returns the client's current nickname.
    #[must_use]
    pub fn nick(&self) -> &str {
        self.core.nick()
    }

    /// Returns the authenticated account name when known.
    #[must_use]
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    /// Reports whether the client is marked away.
    #[must_use]
    pub const fn away(&self) -> bool {
        self.core.away()
    }

    /// Iterates over the client's user modes in unspecified order.
    pub fn user_modes(&self) -> impl Iterator<Item = char> + '_ {
        self.core.user_modes()
    }

    /// Returns a joined channel using the server's casemapping.
    #[must_use]
    pub fn channel(&self, name: &str) -> Option<&Channel> {
        self.core.channel(name)
    }

    /// Iterates over joined channels in unspecified order.
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.core.channels()
    }

    /// Returns a known user using the server's casemapping.
    #[must_use]
    pub fn user(&self, nick: &str) -> Option<&User> {
        self.core.user(nick)
    }

    /// Iterates over users currently reported online through `MONITOR`.
    pub fn monitor_online(&self) -> impl Iterator<Item = &User> {
        self.core.monitor_online()
    }
}
