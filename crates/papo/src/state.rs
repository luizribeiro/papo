use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use papo_core::{Capabilities, Channel, Feature, Session, UnsupportedReason, User};
use papo_proto::{Casemapping, Isupport, NAME_ALIASES, NameKind};

/// An immutable snapshot of an IRC client's known state.
///
/// A snapshot is at least as new as the last event delivered. Events can be
/// applied in order using their self-contained payloads.
#[derive(Clone, Debug)]
pub struct State {
    core: papo_core::State,
    pub(crate) caps: Capabilities,
    pub(crate) isupport: Arc<Isupport>,
    account: Option<String>,
    unsupported: BTreeMap<Feature, UnsupportedReason>,
    connected: bool,
    generation: u64,
}

impl State {
    pub(crate) fn from_session(session: &Session, connected: bool) -> Self {
        let unsupported = FEATURES
            .into_iter()
            .filter_map(|feature| {
                session
                    .supports(feature)
                    .err()
                    .map(|reason| (feature, reason))
            })
            .collect();
        Self {
            core: session.state().clone(),
            caps: session.caps().clone(),
            isupport: Arc::new(session.isupport().clone()),
            account: session.account().map(str::to_owned),
            unsupported,
            connected,
            generation: session.state_generation(),
        }
    }

    /// Reports whether the current connection supports `feature`.
    #[must_use]
    pub fn supports(&self, feature: Feature) -> bool {
        !self.unsupported.contains_key(&feature)
    }

    /// Iterates over the optional behaviors supported by the current connection.
    pub fn features(&self) -> impl Iterator<Item = Feature> + '_ {
        FEATURES
            .into_iter()
            .filter(|feature| self.supports(*feature))
    }

    pub(crate) fn unsupported_reason(&self, feature: Feature) -> Option<UnsupportedReason> {
        self.unsupported.get(&feature).copied()
    }

    pub(crate) const fn matches(&self, session: &Session, connected: bool) -> bool {
        self.generation == session.state_generation() && self.connected == connected
    }

    /// Reports whether the client currently has an open transport.
    #[must_use]
    pub const fn connected(&self) -> bool {
        self.connected
    }

    /// Reports whether the client currently has an open transport.
    #[must_use]
    pub const fn is_connected(&self) -> bool {
        self.connected
    }

    /// Returns the client's current nickname.
    #[must_use]
    pub fn nick(&self) -> &str {
        self.core.nick()
    }

    /// Reports whether `nick` identifies this client under the server's casemapping.
    #[must_use]
    pub fn is_me(&self, nick: &str) -> bool {
        self.casemapping().eq(self.nick(), nick)
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

    /// Returns the newest read marker time reported for `target`.
    #[must_use]
    pub fn read_marker(&self, target: &str) -> Option<std::time::SystemTime> {
        self.core.read_marker(target).and_then(|marker| marker.time)
    }

    /// Returns the most recently reported account persistence status.
    #[must_use]
    pub const fn persistence(&self) -> Option<papo_core::PersistenceStatus> {
        self.core.persistence()
    }

    /// Returns cached metadata for `target`.
    #[must_use]
    pub fn metadata(&self, target: &str) -> Option<&HashMap<String, papo_core::MetadataValue>> {
        self.core.metadata(target)
    }

    /// Returns metadata keys confirmed in the server-side subscription list.
    #[must_use]
    pub fn metadata_subscriptions(&self) -> &[String] {
        self.core.metadata_subscriptions()
    }

    /// Returns a known user using the server's casemapping.
    #[must_use]
    pub fn user(&self, nick: &str) -> Option<&User> {
        self.core.user(nick)
    }

    /// Iterates over users currently reported online through `MONITOR`.
    ///
    /// Presence from `730` replies is included without requiring `extended-monitor`.
    pub fn monitor_online(&self) -> impl Iterator<Item = &User> {
        self.core.monitor_online()
    }

    /// Returns the capabilities enabled on the current connection.
    #[must_use]
    pub const fn caps(&self) -> &Capabilities {
        &self.caps
    }

    /// Reports whether a capability or its ratified alias is enabled.
    #[must_use]
    pub fn has_cap(&self, name: &str) -> bool {
        self.caps.contains_key(name)
            || NAME_ALIASES.iter().any(|&(kind, draft, ratified)| {
                kind == NameKind::Capability
                    && (name == draft || name == ratified)
                    && (self.caps.contains_key(draft) || self.caps.contains_key(ratified))
            })
    }

    /// Returns the accumulated server feature advertisement.
    #[must_use]
    pub fn isupport(&self) -> &Isupport {
        &self.isupport
    }

    /// Returns the server's nickname and channel casemapping.
    #[must_use]
    pub fn casemapping(&self) -> Casemapping {
        self.isupport.casemapping()
    }

    /// Returns the canonical form of an IRC nickname or channel name.
    #[must_use]
    pub fn casemap(&self, name: &str) -> String {
        self.isupport.casemapping().fold(name).into_owned()
    }
}

const FEATURES: [Feature; 20] = [
    Feature::Replies,
    Feature::Reactions,
    Feature::Typing,
    Feature::Redaction,
    Feature::Multiline,
    Feature::History,
    Feature::ReadMarkers,
    Feature::Metadata,
    Feature::Monitor,
    Feature::SetName,
    Feature::BotMode,
    Feature::AccountRegistration,
    Feature::Persistence,
    Feature::WebPush,
    Feature::ChannelRename,
    Feature::Relay,
    Feature::LabeledResponse,
    Feature::AwayTracking,
    Feature::AccountTracking,
    Feature::HostTracking,
];
