//! Capability names understood by papo.

pub use papo_core::cap::{
    ACCOUNT_NOTIFY, EXTENDED_MONITOR, INVITE_NOTIFY, OPER_TAG, STANDARD_REPLIES, STS,
};
pub use papo_core::cap::{ACCOUNT_REGISTRATION, LABELED_RESPONSE, MULTILINE, SASL};
pub use papo_core::cap::{
    ACCOUNT_TAG, AWAY_NOTIFY, CHGHOST, EXTENDED_JOIN, MESSAGE_TAGS, MULTI_PREFIX, SETNAME,
    USERHOST_IN_NAMES,
};
pub use papo_core::cap::{BATCH, CHATHISTORY, METADATA, READ_MARKER};
pub use papo_core::cap::{CAP_NOTIFY, ECHO_MESSAGE, EVENT_PLAYBACK, SERVER_TIME};
/// Account persistence preferences.
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub const PERSISTENCE: &str = papo_core::cap::PERSISTENCE;
pub use papo_core::cap::{
    CHANNEL_RENAME, EXTENDED_ISUPPORT, MESSAGE_REDACTION, NO_IMPLICIT_NAMES, PRE_AWAY, WHOAMI,
};
/// Relayed messages.
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub const RELAYMSG: &str = papo_core::cap::RELAYMSG;
/// Web Push subscriptions.
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub const WEBPUSH: &str = papo_core::cap::WEBPUSH;
