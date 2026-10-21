//! Capability names understood by papo.

pub use papo_core::cap::{
    ACCOUNT_TAG, AWAY_NOTIFY, CHGHOST, EXTENDED_JOIN, MESSAGE_TAGS, MULTI_PREFIX, SETNAME,
    USERHOST_IN_NAMES,
};
pub use papo_core::cap::{CAP_NOTIFY, ECHO_MESSAGE, EVENT_PLAYBACK, SERVER_TIME};
/// Batched messages.
pub const BATCH: &str = papo_core::cap::BATCH;
/// Account-change notifications.
pub const ACCOUNT_NOTIFY: &str = papo_core::cap::ACCOUNT_NOTIFY;
/// Extended monitor replies.
pub const EXTENDED_MONITOR: &str = papo_core::cap::EXTENDED_MONITOR;
/// Invitation notifications.
pub const INVITE_NOTIFY: &str = papo_core::cap::INVITE_NOTIFY;
/// Structured standard replies.
pub const STANDARD_REPLIES: &str = papo_core::cap::STANDARD_REPLIES;
/// Labelled request correlation.
pub const LABELED_RESPONSE: &str = papo_core::cap::LABELED_RESPONSE;
/// Multiline messages.
pub const MULTILINE: &str = papo_core::cap::MULTILINE;
/// Operator tags on messages.
pub const OPER_TAG: &str = papo_core::cap::OPER_TAG;
/// Client metadata.
pub const METADATA: &str = papo_core::cap::METADATA;
/// SASL authentication.
pub const SASL: &str = papo_core::cap::SASL;
/// Strict Transport Security upgrades.
pub const STS: &str = papo_core::cap::STS;
/// Account registration.
pub const ACCOUNT_REGISTRATION: &str = papo_core::cap::ACCOUNT_REGISTRATION;
/// Message history queries.
pub const CHATHISTORY: &str = papo_core::cap::CHATHISTORY;
/// Channel read markers.
pub const READ_MARKER: &str = papo_core::cap::READ_MARKER;
/// Account persistence preferences.
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub const PERSISTENCE: &str = papo_core::cap::PERSISTENCE;
/// Channel renaming.
pub const CHANNEL_RENAME: &str = papo_core::cap::CHANNEL_RENAME;
/// Relayed messages.
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub const RELAYMSG: &str = papo_core::cap::RELAYMSG;
/// Message redaction.
pub const MESSAGE_REDACTION: &str = papo_core::cap::MESSAGE_REDACTION;
/// Web Push subscriptions.
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub const WEBPUSH: &str = papo_core::cap::WEBPUSH;
/// Away state while connecting.
pub const PRE_AWAY: &str = papo_core::cap::PRE_AWAY;
/// Suppression of implicit names replies.
pub const NO_IMPLICIT_NAMES: &str = papo_core::cap::NO_IMPLICIT_NAMES;
/// Current connection identity discovery.
pub const WHOAMI: &str = papo_core::cap::WHOAMI;
/// Extended ISUPPORT discovery.
pub const EXTENDED_ISUPPORT: &str = papo_core::cap::EXTENDED_ISUPPORT;
