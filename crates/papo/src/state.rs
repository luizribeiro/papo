use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use papo_core::{Capabilities, Channel, Feature, Session, UnsupportedReason, User};
use papo_proto::{AccountName, Casemapping, ChannelName, Mask, Nick};

use crate::Isupport;

/// An immutable snapshot of an IRC client's known state.
///
/// A snapshot is at least as new as the last event delivered. Events can be
/// applied in order using their self-contained payloads.
#[derive(Clone, Debug)]
pub struct State {
    core: papo_core::State,
    pub(crate) caps: Capabilities,
    pub(crate) isupport: Arc<Isupport>,
    account: Option<AccountName>,
    metadata_subscriptions: Vec<crate::MetadataKey>,
    #[cfg(feature = "unstable")]
    bouncer_network: Option<papo_core::NetworkId>,
    unsupported: BTreeMap<Feature, UnsupportedReason>,
    connected: bool,
    generation: u64,
}

impl State {
    pub(crate) fn from_session(session: &Session, connected: bool) -> Self {
        let unsupported = FEATURES
            .iter()
            .copied()
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
            isupport: Arc::new(Isupport::from_proto(session.isupport().clone())),
            account: session
                .account()
                .map(|account| session.state().casemapping().account(account)),
            metadata_subscriptions: session
                .state()
                .metadata_subscriptions()
                .iter()
                .map(crate::MetadataKey::from)
                .collect(),
            #[cfg(feature = "unstable")]
            bouncer_network: match session.isupport().get("BOUNCER_NETID") {
                papo_proto::Advertised::Value(id) => Some(papo_core::NetworkId::new(id)),
                _ => None,
            },
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
            .iter()
            .copied()
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
    pub const fn is_connected(&self) -> bool {
        self.connected
    }

    /// Returns the client's current nickname.
    #[must_use]
    pub fn nick(&self) -> &Nick {
        self.core.nick()
    }

    /// Builds a nickname with the current or last-known server-defined comparison rules.
    #[must_use]
    pub fn nickname(&self, text: impl Into<String>) -> Nick {
        self.casemapping().nick(text)
    }

    /// Reports whether `nick` identifies this client under the server's casemapping.
    #[must_use]
    pub fn is_me(&self, nick: impl AsRef<str>) -> bool {
        self.casemapping().eq(self.nick(), nick.as_ref())
    }

    /// Returns the authenticated account name when known.
    #[must_use]
    pub fn account(&self) -> Option<&AccountName> {
        self.account.as_ref()
    }

    /// Builds an account name with the current or last-known server-defined comparison rules.
    #[must_use]
    pub fn account_name(&self, text: impl Into<String>) -> AccountName {
        self.casemapping().account(text)
    }

    /// Reports whether the client is marked away.
    #[must_use]
    pub const fn away(&self) -> bool {
        self.core.away()
    }

    /// Returns the client's confirmed away message.
    ///
    /// `None` means the client is not away or the server confirmed away state without a message
    /// sent through [`crate::Client::set_away`].
    #[must_use]
    pub fn away_message(&self) -> Option<&str> {
        self.core.away_message()
    }

    /// Iterates over the client's user modes in unspecified order.
    pub fn user_modes(&self) -> impl Iterator<Item = char> + '_ {
        self.core.user_modes()
    }

    /// Returns a joined channel using the server's casemapping.
    #[must_use]
    pub fn channel(&self, name: impl AsRef<str>) -> Option<&Channel> {
        self.core.channel(name)
    }

    /// Builds a channel name with the current or last-known server-defined comparison rules.
    #[must_use]
    pub fn channel_name(&self, text: impl Into<String>) -> ChannelName {
        self.casemapping().channel(text)
    }

    /// Iterates over joined channels in unspecified order.
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.core.channels()
    }

    /// Returns the upstream network bound to this bouncer connection.
    #[must_use]
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub const fn bouncer_network(&self) -> Option<&papo_core::NetworkId> {
        self.bouncer_network.as_ref()
    }

    /// Iterates over upstream networks reported by the bouncer.
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub fn bouncer_networks(&self) -> impl Iterator<Item = &papo_core::BouncerNetwork> {
        self.core.bouncer_networks()
    }

    /// Returns the newest read marker time reported for `target`.
    #[must_use]
    pub fn read_marker(&self, target: impl AsRef<str>) -> Option<std::time::SystemTime> {
        self.core.read_marker(target).and_then(|marker| marker.time)
    }

    /// Returns the most recently reported account persistence status.
    #[must_use]
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub fn persistence(&self) -> Option<crate::PersistenceStatus> {
        self.core
            .persistence()
            .and_then(crate::PersistenceStatus::from_core)
    }

    /// Returns cached metadata for `target`.
    #[must_use]
    pub fn metadata(
        &self,
        target: impl AsRef<str>,
    ) -> Option<HashMap<crate::MetadataKey, crate::MetadataValue>> {
        self.core.metadata(target).map(|values| {
            values
                .iter()
                .map(|(key, value)| {
                    (
                        crate::MetadataKey::from(key),
                        crate::MetadataValue::from_core(value),
                    )
                })
                .collect()
        })
    }

    /// Returns metadata keys confirmed in the server-side subscription list.
    #[must_use]
    pub fn metadata_subscriptions(&self) -> &[crate::MetadataKey] {
        &self.metadata_subscriptions
    }

    /// Returns a known user using the server's casemapping.
    #[must_use]
    pub fn user(&self, nick: impl AsRef<str>) -> Option<&User> {
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
        self.caps
            .keys()
            .any(|enabled| papo_core::cap::canonical(enabled) == papo_core::cap::canonical(name))
    }

    /// Returns the accumulated server feature advertisement.
    #[must_use]
    pub fn isupport(&self) -> &Isupport {
        &self.isupport
    }

    /// Returns the HTTPS endpoint advertised for file uploads.
    ///
    /// Uploads use an HTTP `POST` whose response supplies the uploaded URL in its `Location`
    /// header. The caller authenticates that request with its own credentials; papo does not
    /// expose authentication secrets. Non-HTTPS endpoints are ignored.
    #[must_use]
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub fn filehost(&self) -> Option<&str> {
        self.isupport.filehost()
    }

    /// Returns the server-defined name comparison rules.
    ///
    /// While disconnected, these are the last advertised rules or the builder's seed if no
    /// connection has advertised rules yet.
    #[must_use]
    pub fn casemapping(&self) -> Casemapping {
        self.core.casemapping()
    }

    /// Builds a mask with the current or last-known server-defined comparison rules.
    #[must_use]
    pub fn mask(&self, text: impl Into<String>) -> Mask {
        self.casemapping().mask(text)
    }
}

const FEATURES: &[Feature] = &[
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
    #[cfg(feature = "unstable")]
    Feature::Bouncer,
    #[cfg(feature = "unstable")]
    Feature::Persistence,
    #[cfg(feature = "unstable")]
    Feature::WebPush,
    Feature::ChannelRename,
    #[cfg(feature = "unstable")]
    Feature::Relay,
    Feature::LabeledResponse,
    Feature::AwayTracking,
    Feature::AccountTracking,
    Feature::HostTracking,
];

#[cfg(test)]
mod name_tests {
    use papo_core::{Config, Session};

    use super::State;
    use crate::Casemapping;

    #[test]
    fn builds_every_name_with_snapshot_rules() {
        let session = Session::new(
            Config::default()
                .casemapping(Casemapping::Ascii)
                .nick("papo[bot]"),
        );
        let state = State::from_session(&session, false);

        let nick = state.nickname("Alice[bot]");
        let channel = state.channel_name("#Papo[dev]");
        let account = state.account_name("Alice[bot]");
        let mask = state.mask("*!alice@[gateway]");

        for mapping in [
            nick.casemapping(),
            channel.casemapping(),
            account.casemapping(),
            mask.casemapping(),
        ] {
            assert_eq!(mapping, Casemapping::Ascii);
        }
        assert_eq!(nick, "alice[bot]");
        assert_ne!(nick, "alice{bot}");
        assert_eq!(channel, "#papo[dev]");
        assert_ne!(channel, "#papo{dev}");
        assert_eq!(account, "alice[bot]");
        assert_ne!(account, "alice{bot}");
        assert_eq!(mask, "*!ALICE@[gateway]");
        assert_ne!(mask, "*!alice@{gateway}");
    }
}

#[cfg(all(test, feature = "unstable"))]
mod tests {
    use crate::Isupport;

    #[test]
    fn accepts_both_filehost_names_and_requires_https() {
        for name in ["soju.im/FILEHOST", "draft/FILEHOST"] {
            let mut inner = papo_proto::Isupport::default();
            inner.apply_tokens([format!("{name}=https://files.example.net/upload")]);
            let isupport = Isupport::from_proto(inner.clone());
            assert_eq!(
                isupport.filehost(),
                Some("https://files.example.net/upload")
            );

            inner.apply_tokens([format!("{name}=http://files.example.net/upload")]);
            assert_eq!(Isupport::from_proto(inner).filehost(), None);
        }
    }
}
