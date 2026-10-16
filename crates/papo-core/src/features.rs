use crate::Session;

/// Optional IRC behavior understood by the session.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Feature {
    /// Message replies.
    Replies,
    /// Message reactions.
    Reactions,
    /// Typing notifications.
    Typing,
    /// Message redaction.
    Redaction,
    /// Multiline messages.
    Multiline,
    /// Chat history queries.
    History,
    /// Read markers.
    ReadMarkers,
    /// Server-side metadata.
    Metadata,
    /// Server-side presence monitoring.
    Monitor,
    /// Real-name changes.
    SetName,
    /// Server-advertised bot user mode.
    BotMode,
    /// Account registration.
    AccountRegistration,
    /// Account persistence.
    Persistence,
    /// Web Push notifications.
    WebPush,
    /// Channel renaming.
    ChannelRename,
    /// Relay messages.
    Relay,
    /// Labelled response correlation.
    LabeledResponse,
}

impl Feature {
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
            Self::Monitor | Self::BotMode => &[],
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

impl Session {
    /// Reports whether this connection supports an optional IRC behavior.
    ///
    /// # Errors
    ///
    /// Returns the reason the feature is unavailable on this connection.
    pub fn supports(&self, feature: Feature) -> Result<(), UnsupportedReason> {
        feature
            .caps()
            .iter()
            .any(|name| self.caps.enabled().contains_key(*name))
            .then_some(())
            .ok_or(UnsupportedReason::NotOffered)
    }
}
