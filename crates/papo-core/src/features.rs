use papo_proto::Advertised;

use crate::{RequestError, Session, cap};

macro_rules! feature_table {
    ($consumer:ident) => {
        $consumer! {
            /// Published `IRCv3` capability change notifications, requiring `cap-notify`.
            [] CapabilityNotifications => "capability notifications", [Some(cap::CAP_NOTIFY), None];
            /// Published `IRCv3` client-only message tags, requiring `message-tags`.
            [] MessageTags => "client-only message tags", [Some(cap::MESSAGE_TAGS), None];
            /// Published `IRCv3` server timestamps, requiring `server-time`.
            [] ServerTime => "server timestamps", [Some(cap::SERVER_TIME), None];
            /// Published `IRCv3` message batches, requiring `batch`.
            [] Batches => "message batches", [Some(cap::BATCH), None];
            /// `IRCv3` draft event playback, requiring `draft/event-playback`.
            [] EventPlayback => "event playback", [Some(cap::EVENT_PLAYBACK), None];
            /// Published `IRCv3` message echoes, requiring `echo-message`.
            [] EchoMessages => "message echoes", [Some(cap::ECHO_MESSAGE), None];
            /// Published `IRCv3` replies, requiring `message-tags` and an allowed `+reply` or
            /// `+draft/reply` client tag.
            [] Replies => "replies", [Some(cap::MESSAGE_TAGS), None];
            /// `IRCv3` draft message reactions, requiring `message-tags`, an allowed
            /// `+draft/react` tag, and an allowed reply tag.
            [] Reactions => "reactions", [Some(cap::MESSAGE_TAGS), None];
            /// Published `IRCv3` typing notifications, requiring `message-tags` and an allowed
            /// `+typing` client tag.
            [] Typing => "typing notifications", [Some(cap::MESSAGE_TAGS), None];
            /// `IRCv3` draft message redaction, requiring `draft/message-redaction`.
            [] Redaction => cap::MESSAGE_REDACTION.constant_str(), [Some(cap::MESSAGE_REDACTION), None];
            /// `IRCv3` draft multiline messages, requiring `draft/multiline`.
            [] Multiline => "multiline messages", [Some(cap::MULTILINE), None];
            /// `IRCv3` draft chat history, requiring `draft/chathistory`.
            [] History => cap::alias(cap::CHATHISTORY.constant_str(), 0), [Some(cap::CHATHISTORY), None];
            /// `IRCv3` draft read markers, requiring `draft/read-marker`.
            [] ReadMarkers => cap::alias(cap::READ_MARKER.constant_str(), 0), [Some(cap::READ_MARKER), None];
            /// `IRCv3` draft metadata, requiring `draft/metadata-2`.
            ///
            /// Papo speaks Ergo's `draft/metadata-3` semantics; the published specification
            /// currently uses `draft/metadata-2`.
            [] Metadata => "metadata", [Some(cap::METADATA), None];
            /// The published IRC `MONITOR` extension, advertised through the `MONITOR`
            /// ISUPPORT token rather than a capability.
            [] Monitor => "monitoring", [None, None];
            /// Published `IRCv3` real-name changes, requiring `setname`.
            [] SetName => "real-name changes", [Some(cap::SETNAME), None];
            /// Published `IRCv3` bot mode, advertised through the `BOT` ISUPPORT token.
            [] BotMode => "bot mode", [None, None];
            /// `IRCv3` draft account registration, requiring `draft/account-registration`.
            [] AccountCreation => "account creation", [Some(cap::ACCOUNT_REGISTRATION), None];
            /// Upstream network management on an IRC bouncer.
            [cfg(feature = "unstable")]
            Bouncer => "bouncer networks", [Some(cap::BOUNCER_NETWORKS), None];
            /// `IRCv3` draft account persistence, requiring `draft/persistence`.
            [cfg(feature = "unstable")]
            Persistence => "persistence", [Some(cap::PERSISTENCE), None];
            /// `IRCv3` draft Web Push, requiring `draft/webpush` or Soju's vendor
            /// capability `soju.im/webpush`.
            [cfg(feature = "unstable")]
            WebPush => "webpush", [Some(cap::WEBPUSH), None];
            /// `IRCv3` draft channel renaming, requiring `draft/channel-rename`.
            [] ChannelRename => "channel rename", [Some(cap::CHANNEL_RENAME), None];
            /// `IRCv3` draft relay messages, requiring `draft/relaymsg`.
            [cfg(feature = "unstable")]
            Relay => "relaymsg", [Some(cap::RELAYMSG), None];
            /// Published `IRCv3` labelled responses, requiring `labeled-response`.
            [] LabeledResponse => cap::LABELED_RESPONSE.constant_str(), [Some(cap::LABELED_RESPONSE), None];
            /// Published `IRCv3` authentication exchange, requiring `sasl`.
            [] Sasl => "SASL authentication", [Some(cap::SASL), None];
            /// Published `IRCv3` live away information, requiring `away-notify`.
            [] AwayTracking => "away tracking", [Some(cap::AWAY_NOTIFY), None];
            /// Published `IRCv3` live account information, requiring both `account-notify`
            /// and `account-tag`.
            [] AccountTracking => "account tracking", [Some(cap::ACCOUNT_NOTIFY), Some(cap::ACCOUNT_TAG)];
            /// Published `IRCv3` extended join details, requiring `extended-join`.
            [] ExtendedJoin => "extended join details", [Some(cap::EXTENDED_JOIN), None];
            /// Published `IRCv3` complete membership prefixes, requiring `multi-prefix`.
            [] MultiPrefix => "complete membership prefixes", [Some(cap::MULTI_PREFIX), None];
            /// Published `IRCv3` user and host fields in names replies, requiring
            /// `userhost-in-names`.
            [] UserhostInNames => "user and host fields in names replies", [Some(cap::USERHOST_IN_NAMES), None];
            /// Published `IRCv3` live username and hostname information, requiring `chghost`.
            [] HostTracking => "host tracking", [Some(cap::CHGHOST), None];
            /// Published `IRCv3` invitation notifications, requiring `invite-notify`.
            [] InviteTracking => "invite tracking", [Some(cap::INVITE_NOTIFY), None];
            /// Extended monitor replies, requiring `extended-monitor`.
            [] ExtendedMonitor => "extended monitor replies", [Some(cap::EXTENDED_MONITOR), None];
            /// Published `IRCv3` structured replies, requiring `standard-replies`.
            [] StandardReplies => "structured replies", [Some(cap::STANDARD_REPLIES), None];
            /// `IRCv3` draft operator tags, requiring `draft/oper-tag`.
            [] OperatorTags => "operator tags", [Some(cap::OPER_TAG), None];
            /// `IRCv3` draft registration-time away state, requiring `draft/pre-away`.
            [] PreAway => "registration-time away state", [Some(cap::PRE_AWAY), None];
            /// Suppression of implicit names replies, requiring `no-implicit-names`.
            [] NoImplicitNames => "suppression of implicit names replies", [Some(cap::NO_IMPLICIT_NAMES), None];
            /// `IRCv3` draft connection identity discovery, requiring `draft/whoami`.
            [] WhoAmI => "connection identity discovery", [Some(cap::WHOAMI), None];
            /// `IRCv3` draft extended ISUPPORT discovery, requiring `draft/extended-isupport`.
            [] ExtendedIsupport => "extended ISUPPORT discovery", [Some(cap::EXTENDED_ISUPPORT), None];
            /// Published IRC half-operator channel membership, advertised through `PREFIX`.
            [] Halfop => "half-operator privileges", [None, None];
            /// Published IRC channel ban exceptions, advertised through `EXCEPTS`.
            [] Excepts => "ban exceptions", [None, None];
            /// Published IRC channel invite exceptions, advertised through `INVEX`.
            [] Invex => "invite exceptions", [None, None];
            /// A published IRC dedicated quiet list in the first `CHANMODES` group.
            [] Quiet => "quiet list", [None, None];
            /// Published `IRCv3` account extended bans, advertised through `EXTBAN` and
            /// `ACCOUNTEXTBAN`.
            [] AccountExtban => "account extended bans", [None, None];
        }
    };
}

macro_rules! feature_enum {
    ($($(#[$docs:meta])* [$($gate:meta),*] $variant:ident => $name:expr, $caps:expr;)*) => {
        /// Optional IRC behavior understood by the session.
        ///
        /// A Feature is something your app can use or rely on that the server may not offer.
        /// Caps that only change wire details get no Feature.
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[non_exhaustive]
        pub enum Feature {
            $($(#[$docs])* $(#[$gate])* $variant,)*
        }

        impl Feature {
            #[doc(hidden)]
            pub const ALL: &'static [Self] = &[
                $($(#[$gate])* Self::$variant,)*
            ];

            /// Returns a stable human-readable feature name.
            #[must_use]
            pub fn name(self) -> &'static str {
                match self {
                    $($(#[$gate])* Self::$variant => $name,)*
                }
            }

            fn caps(self) -> [Option<cap::Cap>; 2] {
                match self {
                    $($(#[$gate])* Self::$variant => $caps,)*
                }
            }
        }
    };
}

feature_table!(feature_enum);

impl Feature {
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
                    .any(|prefix| prefix.mode() == 'h')
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
