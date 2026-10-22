use papo_proto::Advertised;

use crate::{RequestError, Session, cap};

/// Optional IRC behavior understood by the session.
///
/// A Feature is something your app can use or rely on that the server may not offer.
/// Caps that only change wire details get no Feature.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Feature {
    /// Published `IRCv3` client-only message tags, requiring `message-tags`.
    MessageTags,
    /// Published `IRCv3` replies, requiring `message-tags` and an allowed `+reply` or
    /// `+draft/reply` client tag.
    Replies,
    /// `IRCv3` draft message reactions, requiring `message-tags`, an allowed
    /// `+draft/react` tag, and an allowed reply tag.
    Reactions,
    /// Published `IRCv3` typing notifications, requiring `message-tags` and an allowed
    /// `+typing` client tag.
    Typing,
    /// `IRCv3` draft message redaction, requiring `draft/message-redaction`.
    Redaction,
    /// `IRCv3` draft multiline messages, requiring `draft/multiline`.
    Multiline,
    /// `IRCv3` draft chat history, requiring `draft/chathistory`.
    History,
    /// `IRCv3` draft read markers, requiring `draft/read-marker`.
    ReadMarkers,
    /// `IRCv3` draft metadata, requiring `draft/metadata-2`.
    ///
    /// Papo speaks Ergo's `draft/metadata-3` semantics; the published specification
    /// currently uses `draft/metadata-2`.
    Metadata,
    /// The published IRC `MONITOR` extension, advertised through the `MONITOR`
    /// ISUPPORT token rather than a capability.
    Monitor,
    /// Published `IRCv3` real-name changes, requiring `setname`.
    SetName,
    /// Published `IRCv3` bot mode, advertised through the `BOT` ISUPPORT token.
    BotMode,
    /// `IRCv3` draft account registration, requiring `draft/account-registration`.
    AccountCreation,
    /// Upstream network management on an IRC bouncer.
    #[cfg(feature = "unstable")]
    Bouncer,
    /// `IRCv3` draft account persistence, requiring `draft/persistence`.
    #[cfg(feature = "unstable")]
    Persistence,
    /// `IRCv3` draft Web Push, requiring `draft/webpush` or Soju's vendor
    /// capability `soju.im/webpush`.
    #[cfg(feature = "unstable")]
    WebPush,
    /// `IRCv3` draft channel renaming, requiring `draft/channel-rename`.
    ChannelRename,
    /// `IRCv3` draft relay messages, requiring `draft/relaymsg`.
    #[cfg(feature = "unstable")]
    Relay,
    /// Published `IRCv3` labelled responses, requiring `labeled-response`.
    LabeledResponse,
    /// Published `IRCv3` live away information, requiring `away-notify`.
    AwayTracking,
    /// Published `IRCv3` live account information, requiring both `account-notify`
    /// and `account-tag`.
    AccountTracking,
    /// Published `IRCv3` live username and hostname information, requiring `chghost`.
    HostTracking,
    /// Published `IRCv3` invitation notifications, requiring `invite-notify`.
    InviteTracking,
    /// Published IRC half-operator channel membership, advertised through `PREFIX`.
    Halfop,
    /// Published IRC channel ban exceptions, advertised through `EXCEPTS`.
    Excepts,
    /// Published IRC channel invite exceptions, advertised through `INVEX`.
    Invex,
    /// A published IRC dedicated quiet list in the first `CHANMODES` group.
    Quiet,
    /// Published `IRCv3` account extended bans, advertised through `EXTBAN` and
    /// `ACCOUNTEXTBAN`.
    AccountExtban,
}

impl Feature {
    #[doc(hidden)]
    pub const ALL: &'static [Self] = &[
        Self::MessageTags,
        Self::Replies,
        Self::Reactions,
        Self::Typing,
        Self::Redaction,
        Self::Multiline,
        Self::History,
        Self::ReadMarkers,
        Self::Metadata,
        Self::Monitor,
        Self::SetName,
        Self::BotMode,
        Self::AccountCreation,
        #[cfg(feature = "unstable")]
        Self::Bouncer,
        #[cfg(feature = "unstable")]
        Self::Persistence,
        #[cfg(feature = "unstable")]
        Self::WebPush,
        Self::ChannelRename,
        #[cfg(feature = "unstable")]
        Self::Relay,
        Self::LabeledResponse,
        Self::AwayTracking,
        Self::AccountTracking,
        Self::HostTracking,
        Self::InviteTracking,
        Self::Halfop,
        Self::Excepts,
        Self::Invex,
        Self::Quiet,
        Self::AccountExtban,
    ];

    /// Returns a stable human-readable feature name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::MessageTags => "client-only message tags",
            Self::Replies => "replies",
            Self::Reactions => "reactions",
            Self::Typing => "typing notifications",
            Self::Redaction => cap::MESSAGE_REDACTION.constant_str(),
            Self::Multiline => "multiline messages",
            Self::History => cap::alias(cap::CHATHISTORY.constant_str(), 0),
            Self::ReadMarkers => cap::alias(cap::READ_MARKER.constant_str(), 0),
            Self::Metadata => "metadata",
            Self::Monitor => "monitoring",
            Self::SetName => "real-name changes",
            Self::BotMode => "bot mode",
            Self::AccountCreation => "account creation",
            #[cfg(feature = "unstable")]
            Self::Bouncer => "bouncer networks",
            #[cfg(feature = "unstable")]
            Self::Persistence => "persistence",
            #[cfg(feature = "unstable")]
            Self::WebPush => "webpush",
            Self::ChannelRename => "channel rename",
            #[cfg(feature = "unstable")]
            Self::Relay => "relaymsg",
            Self::LabeledResponse => cap::LABELED_RESPONSE.constant_str(),
            Self::AwayTracking => "away tracking",
            Self::AccountTracking => "account tracking",
            Self::HostTracking => "host tracking",
            Self::InviteTracking => "invite tracking",
            Self::Halfop => "half-operator privileges",
            Self::Excepts => "ban exceptions",
            Self::Invex => "invite exceptions",
            Self::Quiet => "quiet list",
            Self::AccountExtban => "account extended bans",
        }
    }

    fn caps(self) -> [Option<cap::Cap>; 2] {
        match self {
            Self::MessageTags | Self::Replies | Self::Reactions | Self::Typing => {
                [Some(cap::MESSAGE_TAGS), None]
            }
            Self::Redaction => [Some(cap::MESSAGE_REDACTION), None],
            Self::Multiline => [Some(cap::MULTILINE), None],
            Self::History => [Some(cap::CHATHISTORY), None],
            Self::ReadMarkers => [Some(cap::READ_MARKER), None],
            Self::Metadata => [Some(cap::METADATA), None],
            Self::SetName => [Some(cap::SETNAME), None],
            Self::AccountCreation => [Some(cap::ACCOUNT_REGISTRATION), None],
            #[cfg(feature = "unstable")]
            Self::Bouncer => [Some(cap::BOUNCER_NETWORKS), None],
            #[cfg(feature = "unstable")]
            Self::Persistence => [Some(cap::PERSISTENCE), None],
            #[cfg(feature = "unstable")]
            Self::WebPush => [Some(cap::WEBPUSH), None],
            Self::ChannelRename => [Some(cap::CHANNEL_RENAME), None],
            #[cfg(feature = "unstable")]
            Self::Relay => [Some(cap::RELAYMSG), None],
            Self::LabeledResponse => [Some(cap::LABELED_RESPONSE), None],
            Self::AwayTracking => [Some(cap::AWAY_NOTIFY), None],
            Self::AccountTracking => [Some(cap::ACCOUNT_NOTIFY), Some(cap::ACCOUNT_TAG)],
            Self::HostTracking => [Some(cap::CHGHOST), None],
            Self::InviteTracking => [Some(cap::INVITE_NOTIFY), None],
            Self::Monitor
            | Self::BotMode
            | Self::Halfop
            | Self::Excepts
            | Self::Invex
            | Self::Quiet
            | Self::AccountExtban => [None, None],
        }
    }

    const fn tags(self) -> &'static [&'static str] {
        match self {
            Self::Replies => &["+reply", "+draft/reply"],
            Self::Reactions => &["+draft/react"],
            Self::Typing => &["+typing"],
            _ => &[],
        }
    }
}

impl std::fmt::Display for Feature {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Why an optional IRC behavior is unavailable.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum UnsupportedReason {
    /// The server did not advertise the required capability or token.
    NotOffered,
    /// The client explicitly disabled the required capability.
    DisabledByClient,
    /// The server denies the required client-only message tag.
    TagDenied,
    /// The server advertised a limit of zero for the feature.
    LimitZero,
}

/// A requested optional IRC behavior that is unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Unsupported {
    /// The requested behavior.
    pub feature: Feature,
    /// Why the behavior is unavailable.
    pub reason: UnsupportedReason,
}

impl Unsupported {
    /// Creates an unavailable-feature description.
    #[must_use]
    pub const fn new(feature: Feature, reason: UnsupportedReason) -> Self {
        Self { feature, reason }
    }
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} unavailable: {}",
            self.feature.name(),
            match self.reason {
                UnsupportedReason::NotOffered => "the server did not offer it",
                UnsupportedReason::DisabledByClient => "it was disabled by the client",
                UnsupportedReason::TagDenied => "the server denies its client-only tag",
                UnsupportedReason::LimitZero => "the server advertised a zero limit",
            }
        )
    }
}

impl std::error::Error for Unsupported {}

impl Session {
    pub(crate) fn supported_features(&self) -> std::collections::BTreeSet<Feature> {
        Feature::ALL
            .iter()
            .copied()
            .filter(|&feature| self.supports(feature).is_ok())
            .collect()
    }

    pub(crate) fn require_feature(&self, feature: Feature) -> Result<(), RequestError> {
        self.supports(feature)
            .map_err(|reason| RequestError::Unsupported(Unsupported::new(feature, reason)))
    }

    /// Reports whether this connection supports an optional IRC behavior.
    ///
    /// # Errors
    ///
    /// Returns the reason the feature is unavailable on this connection.
    pub fn supports(&self, feature: Feature) -> Result<(), UnsupportedReason> {
        #[cfg(feature = "unstable")]
        if matches!(
            feature,
            Feature::Bouncer | Feature::Persistence | Feature::WebPush | Feature::Relay
        ) && !self.config.enabled_features.contains(&feature)
        {
            return Err(UnsupportedReason::DisabledByClient);
        }
        let support = self.isupport.value();
        match feature {
            Feature::Monitor => match support.get("MONITOR") {
                Advertised::Absent => return Err(UnsupportedReason::NotOffered),
                Advertised::Value("0") => return Err(UnsupportedReason::LimitZero),
                Advertised::Bare | Advertised::Value(_) => return Ok(()),
            },
            Feature::BotMode => {
                return support
                    .bot_mode()
                    .map(|_| ())
                    .ok_or(UnsupportedReason::NotOffered);
            }
            Feature::Halfop => {
                return support
                    .prefix()
                    .iter()
                    .any(|prefix| prefix.mode == 'h')
                    .then_some(())
                    .ok_or(UnsupportedReason::NotOffered);
            }
            Feature::Excepts => {
                return support
                    .excepts()
                    .map(|_| ())
                    .ok_or(UnsupportedReason::NotOffered);
            }
            Feature::Invex => {
                return support
                    .invex()
                    .map(|_| ())
                    .ok_or(UnsupportedReason::NotOffered);
            }
            Feature::Quiet => {
                return support
                    .chanmodes()
                    .filter(|groups| groups.list.contains('q'))
                    .map(|_| ())
                    .ok_or(UnsupportedReason::NotOffered);
            }
            Feature::AccountExtban => {
                return (support.extban().is_some() && !support.accountextban().is_empty())
                    .then_some(())
                    .ok_or(UnsupportedReason::NotOffered);
            }
            _ => {}
        }
        let required = feature.caps();
        let enabled =
            if feature == Feature::AccountTracking {
                required.iter().flatten().all(|name| {
                    cap::advertised_spelling(name.as_str(), self.caps.enabled()).is_some()
                })
            } else {
                required.iter().flatten().any(|name| {
                    cap::advertised_spelling(name.as_str(), self.caps.enabled()).is_some()
                })
            };
        if !enabled {
            let disabled = required.iter().flatten().any(|required| {
                self.config.cap_disabled(required.as_str())
                    && self
                        .caps
                        .advertised()
                        .keys()
                        .any(|offered| offered == required)
            });
            return Err(if disabled {
                UnsupportedReason::DisabledByClient
            } else {
                UnsupportedReason::NotOffered
            });
        }
        let denied = support.clienttagdeny();
        if feature == Feature::Reactions
            && (denied.denies("+draft/react")
                || (denied.denies("+reply") && denied.denies("+draft/reply")))
        {
            return Err(UnsupportedReason::TagDenied);
        }
        if !feature.tags().is_empty() && feature.tags().iter().all(|tag| denied.denies(tag)) {
            return Err(UnsupportedReason::TagDenied);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Feature;
    use crate::cap;

    #[test]
    fn registry_and_feature_capabilities_agree_in_both_directions() {
        for feature in Feature::ALL {
            for required in feature.caps().iter().flatten() {
                let info = cap::ALL
                    .iter()
                    .find(|info| info.name == cap::canonical(required.as_str()))
                    .unwrap();
                assert!(info.features.contains(feature), "{feature:?}: {required}");
            }
        }
        for info in cap::ALL {
            for feature in info.features {
                assert!(
                    feature
                        .caps()
                        .iter()
                        .flatten()
                        .any(|name| cap::canonical(name.as_str()) == info.name),
                    "{}: {feature:?}",
                    info.name
                );
            }
        }
    }
}
