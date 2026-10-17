//! Capability names understood by papo.

/// Capability change notifications.
pub const CAP_NOTIFY: &str = "cap-notify";
/// `IRCv3` message tags.
pub const MESSAGE_TAGS: &str = "message-tags";
/// Server-assigned message timestamps.
pub const SERVER_TIME: &str = "server-time";
/// Batched messages.
pub const BATCH: &str = "batch";
/// Replay of missed events.
pub const EVENT_PLAYBACK: &str = "draft/event-playback";
/// Echoes of sent messages.
pub const ECHO_MESSAGE: &str = "echo-message";
/// Account tags on messages.
pub const ACCOUNT_TAG: &str = "account-tag";
/// Account and real-name fields on joins.
pub const EXTENDED_JOIN: &str = "extended-join";
/// Every membership prefix in names replies.
pub const MULTI_PREFIX: &str = "multi-prefix";
/// User and host fields in names replies.
pub const USERHOST_IN_NAMES: &str = "userhost-in-names";
/// Host-change notifications.
pub const CHGHOST: &str = "chghost";
/// Real-name changes.
pub const SETNAME: &str = "setname";
/// Away-state notifications.
pub const AWAY_NOTIFY: &str = "away-notify";
/// Account-change notifications.
pub const ACCOUNT_NOTIFY: &str = "account-notify";
/// Extended monitor replies.
pub const EXTENDED_MONITOR: &str = "extended-monitor";
/// Invitation notifications.
pub const INVITE_NOTIFY: &str = "invite-notify";
/// Structured standard replies.
pub const STANDARD_REPLIES: &str = "standard-replies";
/// Labelled request correlation.
pub const LABELED_RESPONSE: &str = "labeled-response";
/// Multiline messages.
pub const MULTILINE: &str = "draft/multiline";
/// Operator tags on messages.
pub const OPER_TAG: &str = "draft/oper-tag";
/// Client metadata.
pub const METADATA: &str = "draft/metadata-2";
/// SASL authentication.
pub const SASL: &str = "sasl";
/// Strict Transport Security upgrades.
pub const STS: &str = "sts";
/// Account registration.
pub const ACCOUNT_REGISTRATION: &str = "draft/account-registration";
/// Message history queries.
pub const CHATHISTORY: &str = "draft/chathistory";
/// Channel read markers.
pub const READ_MARKER: &str = "draft/read-marker";
/// Account persistence preferences.
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub const PERSISTENCE: &str = "draft/persistence";
/// Channel renaming.
pub const CHANNEL_RENAME: &str = "draft/channel-rename";
/// Relayed messages.
pub const RELAYMSG: &str = "draft/relaymsg";
/// Message redaction.
pub const MESSAGE_REDACTION: &str = "draft/message-redaction";
/// Web Push subscriptions.
pub const WEBPUSH: &str = "draft/webpush";
/// Away state during registration.
pub const PRE_AWAY: &str = "draft/pre-away";
/// Suppression of implicit names replies.
pub const NO_IMPLICIT_NAMES: &str = "draft/no-implicit-names";
/// Current connection identity discovery.
pub const WHOAMI: &str = "draft/whoami";
/// Extended ISUPPORT discovery.
pub const EXTENDED_ISUPPORT: &str = "draft/extended-isupport";
