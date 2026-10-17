use papo_proto::Advertised;

use crate::{RequestError, Session};

/// Optional IRC behavior understood by the session.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Feature {
    /// `IRCv3` draft replies, requiring `message-tags` and an allowed `+reply` or
    /// `+draft/reply` client tag.
    Replies,
    /// `IRCv3` draft message reactions, requiring `message-tags`, an allowed
    /// `+draft/react` tag, and an allowed reply tag.
    Reactions,
    /// `IRCv3` draft typing notifications, requiring `message-tags` and an allowed
    /// `+typing` client tag.
    Typing,
    /// `IRCv3` draft message redaction, requiring `draft/message-redaction`.
    Redaction,
    /// `IRCv3` draft multiline messages, requiring `draft/multiline`.
    Multiline,
    /// Ratified `IRCv3` chat history, requiring `chathistory` or its
    /// `draft/chathistory` predecessor.
    History,
    /// `IRCv3` draft read markers, requiring `draft/read-marker` or `read-marker`.
    ReadMarkers,
    /// `IRCv3` draft metadata, requiring `draft/metadata-3` or `draft/metadata-2`.
    Metadata,
    /// The ratified IRC `MONITOR` extension, advertised through the `MONITOR`
    /// ISUPPORT token rather than a capability.
    Monitor,
    /// Ratified `IRCv3` real-name changes, requiring `setname`.
    SetName,
    /// Ratified `IRCv3` bot mode, advertised through the `BOT` ISUPPORT token.
    BotMode,
    /// `IRCv3` draft account registration, requiring `draft/account-registration`.
    AccountRegistration,
    /// `IRCv3` draft account persistence, requiring `draft/persistence`.
    Persistence,
    /// `IRCv3` draft Web Push, requiring `draft/webpush` or Soju's vendor
    /// capability `soju.im/webpush`.
    WebPush,
    /// `IRCv3` draft channel renaming, requiring `draft/channel-rename`.
    ChannelRename,
    /// `IRCv3` draft relay messages, requiring `draft/relaymsg`.
    Relay,
    /// Ratified `IRCv3` labelled responses, requiring `labeled-response`.
    LabeledResponse,
    /// Ratified `IRCv3` live away information, requiring `away-notify`.
    AwayTracking,
    /// Ratified `IRCv3` live account information, requiring both `account-notify`
    /// and `account-tag`.
    AccountTracking,
    /// Ratified `IRCv3` live username and hostname information, requiring `chghost`.
    HostTracking,
}

impl Feature {
    pub(crate) const ALL: [Self; 20] = [
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
        Self::AccountRegistration,
        Self::Persistence,
        Self::WebPush,
        Self::ChannelRename,
        Self::Relay,
        Self::LabeledResponse,
        Self::AwayTracking,
        Self::AccountTracking,
        Self::HostTracking,
    ];

    /// Returns a stable human-readable feature name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Replies => "replies",
            Self::Reactions => "reactions",
            Self::Typing => "typing notifications",
            Self::Redaction => "draft/message-redaction",
            Self::Multiline => "multiline messages",
            Self::History => "chathistory",
            Self::ReadMarkers => "read-marker",
            Self::Metadata => "metadata",
            Self::Monitor => "monitoring",
            Self::SetName => "real-name changes",
            Self::BotMode => "bot mode",
            Self::AccountRegistration => "account-registration",
            Self::Persistence => "persistence",
            Self::WebPush => "webpush",
            Self::ChannelRename => "channel rename",
            Self::Relay => "relaymsg",
            Self::LabeledResponse => "labeled-response",
            Self::AwayTracking => "away tracking",
            Self::AccountTracking => "account tracking",
            Self::HostTracking => "host tracking",
        }
    }

    const fn caps(self) -> &'static [&'static str] {
        match self {
            Self::Replies | Self::Reactions | Self::Typing => &["message-tags"],
            Self::Redaction => &["draft/message-redaction"],
            Self::Multiline => &["draft/multiline"],
            Self::History => &["draft/chathistory", "chathistory"],
            Self::ReadMarkers => &["draft/read-marker", "read-marker"],
            Self::Metadata => &["draft/metadata-3", "draft/metadata-2"],
            Self::SetName => &["setname"],
            Self::AccountRegistration => &["draft/account-registration"],
            Self::Persistence => &["draft/persistence"],
            Self::WebPush => &["draft/webpush", "soju.im/webpush"],
            Self::ChannelRename => &["draft/channel-rename"],
            Self::Relay => &["draft/relaymsg"],
            Self::LabeledResponse => &["labeled-response"],
            Self::AwayTracking => &["away-notify"],
            Self::AccountTracking => &["account-notify", "account-tag"],
            Self::HostTracking => &["chghost"],
            Self::Monitor | Self::BotMode => &[],
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

/// Why an optional IRC behavior is unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

impl Session {
    pub(crate) fn supported_features(&self) -> std::collections::BTreeSet<Feature> {
        Feature::ALL
            .into_iter()
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
            _ => {}
        }
        let enabled = if feature == Feature::AccountTracking {
            feature
                .caps()
                .iter()
                .all(|name| self.caps.enabled().contains_key(*name))
        } else {
            feature
                .caps()
                .iter()
                .any(|name| self.caps.enabled().contains_key(*name))
        };
        if !enabled {
            let disabled = feature.caps().iter().any(|required| {
                self.config.cap_disabled(required)
                    && self.caps.advertised().keys().any(|offered| {
                        papo_proto::canonical(offered) == papo_proto::canonical(required)
                    })
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
