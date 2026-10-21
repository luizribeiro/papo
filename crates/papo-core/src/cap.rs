//! IRC capability names and their implementation homes.

use std::{
    borrow::{Borrow, Cow},
    cmp::Ordering,
    hash::{Hash, Hasher},
};

use crate::{Capabilities, Feature};

/// An IRC capability name.
///
/// Draft and final spellings of the same known capability compare and hash as
/// equal. Formatting preserves the spelling supplied by the server or caller.
#[derive(Clone, Debug)]
pub struct Cap(Cow<'static, str>);

impl Cap {
    #[allow(dead_code)]
    const fn new(name: &'static str) -> Self {
        Self(Cow::Borrowed(name))
    }

    /// Returns the capability spelling supplied by the server or caller.
    #[must_use]
    pub const fn as_str(&self) -> &str {
        match &self.0 {
            Cow::Borrowed(name) => name,
            Cow::Owned(name) => name.as_str(),
        }
    }

    fn identity(&self) -> &str {
        canonical(self.as_str())
    }
}

impl From<&str> for Cap {
    fn from(name: &str) -> Self {
        Self(Cow::Owned(name.to_owned()))
    }
}

impl From<String> for Cap {
    fn from(name: String) -> Self {
        Self(Cow::Owned(name))
    }
}

impl std::fmt::Display for Cap {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl PartialEq for Cap {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for Cap {}

impl Borrow<str> for Cap {
    fn borrow(&self) -> &str {
        self.identity()
    }
}

impl PartialOrd for Cap {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Cap {
    fn cmp(&self, other: &Self) -> Ordering {
        self.identity().cmp(other.identity())
    }
}

impl Hash for Cap {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

/// How papo requests a capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request {
    /// Requested whenever the server advertises it, using this spelling.
    Default(&'static str),
    /// Requested only when the corresponding configuration needs it.
    Automatic,
    /// Observed but never requested.
    Never,
    /// Requested only when a preferred capability is unavailable.
    Superseded(&'static str),
}

/// Registry information for one IRC capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapInfo {
    /// Preferred protocol spelling.
    pub name: &'static str,
    /// Other spellings accepted for the same capability, in preference order.
    pub aliases: &'static [&'static str],
    /// Request policy.
    pub request: Request,
    /// Public features implemented using the capability.
    pub features: &'static [Feature],
    /// Modules implementing its behavior.
    pub code: &'static [&'static str],
}

macro_rules! capabilities {
    ($($(#[$meta:meta])* $constant:ident = $value:literal; $doc:literal;
        name: $name:literal; aliases: [$($alias:literal),*]; request: $request:expr;
        features: [$($feature:path),*]; code: [$($code:path),+];)*) => {
        $($(#[$meta])* #[doc = $doc] pub const $constant: &str = $value;)*

        /// Every capability understood by papo.
        pub static ALL: &[CapInfo] = &[$($(#[$meta])* CapInfo {
            name: $name, aliases: &[$($alias),*], request: $request,
            features: &[$($feature),*], code: &[$(stringify!($code)),+],
        },)*];

        $($(#[$meta])* const _: () = { $(#[allow(unused_imports)] use $code as _;)+ };)*
    };
}

capabilities! {
    CAP_NOTIFY = "cap-notify"; "Capability change notifications."; name: "cap-notify"; aliases: []; request: Request::Default("cap-notify"); features: []; code: [crate::caps];
    MESSAGE_TAGS = "message-tags"; "`IRCv3` message tags."; name: "message-tags"; aliases: []; request: Request::Default("message-tags"); features: [Feature::MessageTags, Feature::Replies, Feature::Reactions, Feature::Typing]; code: [crate::features, crate::replies];
    SERVER_TIME = "server-time"; "Server-assigned message timestamps."; name: "server-time"; aliases: []; request: Request::Default("server-time"); features: []; code: [crate::state::read_marker];
    BATCH = "batch"; "Batched messages."; name: "batch"; aliases: []; request: Request::Default("batch"); features: []; code: [crate::batch];
    EVENT_PLAYBACK = "draft/event-playback"; "Replay of missed events."; name: "draft/event-playback"; aliases: []; request: Request::Default("draft/event-playback"); features: []; code: [crate::batch];
    ECHO_MESSAGE = "echo-message"; "Echoes of sent messages."; name: "echo-message"; aliases: []; request: Request::Default("echo-message"); features: []; code: [crate::outgoing, crate::echo];
    ACCOUNT_TAG = "account-tag"; "Account tags on messages."; name: "account-tag"; aliases: []; request: Request::Default("account-tag"); features: [Feature::AccountTracking]; code: [crate::state::presence];
    EXTENDED_JOIN = "extended-join"; "Account and real-name fields on joins."; name: "extended-join"; aliases: []; request: Request::Default("extended-join"); features: []; code: [crate::state::membership];
    MULTI_PREFIX = "multi-prefix"; "Every membership prefix in names replies."; name: "multi-prefix"; aliases: []; request: Request::Default("multi-prefix"); features: []; code: [crate::state::membership];
    USERHOST_IN_NAMES = "userhost-in-names"; "User and host fields in names replies."; name: "userhost-in-names"; aliases: []; request: Request::Default("userhost-in-names"); features: []; code: [crate::state::membership];
    CHGHOST = "chghost"; "Host-change notifications."; name: "chghost"; aliases: []; request: Request::Default("chghost"); features: [Feature::HostTracking]; code: [crate::state::presence];
    SETNAME = "setname"; "Real-name changes."; name: "setname"; aliases: []; request: Request::Default("setname"); features: [Feature::SetName]; code: [crate::state::presence];
    AWAY_NOTIFY = "away-notify"; "Away-state notifications."; name: "away-notify"; aliases: []; request: Request::Default("away-notify"); features: [Feature::AwayTracking]; code: [crate::state::presence];
    ACCOUNT_NOTIFY = "account-notify"; "Account-change notifications."; name: "account-notify"; aliases: []; request: Request::Default("account-notify"); features: [Feature::AccountTracking]; code: [crate::state::presence];
    EXTENDED_MONITOR = "extended-monitor"; "Extended monitor replies."; name: "extended-monitor"; aliases: []; request: Request::Default("extended-monitor"); features: []; code: [crate::monitor];
    INVITE_NOTIFY = "invite-notify"; "Invitation notifications."; name: "invite-notify"; aliases: []; request: Request::Default("invite-notify"); features: []; code: [crate::replies];
    STANDARD_REPLIES = "standard-replies"; "Structured standard replies."; name: "standard-replies"; aliases: []; request: Request::Default("standard-replies"); features: []; code: [crate::refusal, crate::replies];
    LABELED_RESPONSE = "labeled-response"; "Labelled request correlation."; name: "labeled-response"; aliases: []; request: Request::Default("labeled-response"); features: [Feature::LabeledResponse]; code: [crate::outgoing, crate::labels];
    MULTILINE = "draft/multiline"; "Multiline messages."; name: "draft/multiline"; aliases: []; request: Request::Default("draft/multiline"); features: [Feature::Multiline]; code: [crate::outgoing, crate::batch];
    OPER_TAG = "draft/oper-tag"; "Operator tags on messages."; name: "draft/oper-tag"; aliases: []; request: Request::Default("draft/oper-tag"); features: []; code: [crate::replies];
    METADATA = "draft/metadata-2"; "Client metadata."; name: "draft/metadata-3"; aliases: ["draft/metadata-2"]; request: Request::Default("draft/metadata-2"); features: [Feature::Metadata]; code: [crate::metadata, crate::state::metadata];
    METADATA_NOTIFY = "metadata-notify"; "Legacy metadata notifications."; name: "metadata-notify"; aliases: []; request: Request::Superseded("draft/metadata-3"); features: []; code: [crate::metadata, crate::state::metadata];
    SASL = "sasl"; "SASL authentication."; name: "sasl"; aliases: []; request: Request::Automatic; features: []; code: [crate::sasl];
    STS = "sts"; "Strict Transport Security policy."; name: "sts"; aliases: []; request: Request::Never; features: []; code: [crate::sts];
    NOPE = "ergo.chat/nope"; "Ergo capability-negotiation test trap."; name: "ergo.chat/nope"; aliases: []; request: Request::Never; features: []; code: [crate::caps];
    ACCOUNT_REQUIRED = "soju.im/account-required"; "Soju authentication requirement."; name: "soju.im/account-required"; aliases: []; request: Request::Never; features: []; code: [crate::caps];
    ACCOUNT_REGISTRATION = "draft/account-registration"; "Account registration."; name: "draft/account-registration"; aliases: ["account-registration"]; request: Request::Default("draft/account-registration"); features: [Feature::AccountCreation]; code: [crate::account_registration];
    #[cfg(feature = "unstable")] BOUNCER_NETWORKS = "soju.im/bouncer-networks"; "Bouncer network management."; name: "soju.im/bouncer-networks"; aliases: []; request: Request::Default("soju.im/bouncer-networks"); features: [Feature::Bouncer]; code: [crate::bouncer];
    #[cfg(feature = "unstable")] BOUNCER_NETWORKS_NOTIFY = "soju.im/bouncer-networks-notify"; "Bouncer network change notifications."; name: "soju.im/bouncer-networks-notify"; aliases: []; request: Request::Default("soju.im/bouncer-networks-notify"); features: []; code: [crate::bouncer];
    CHATHISTORY = "draft/chathistory"; "Message history queries."; name: "chathistory"; aliases: ["draft/chathistory"]; request: Request::Default("chathistory"); features: [Feature::History]; code: [crate::history, crate::batch];
    READ_MARKER = "draft/read-marker"; "Channel read markers."; name: "draft/read-marker"; aliases: ["read-marker"]; request: Request::Default("draft/read-marker"); features: [Feature::ReadMarkers]; code: [crate::history, crate::state::read_marker];
    #[cfg(feature = "unstable")] PERSISTENCE = "draft/persistence"; "Account persistence preferences."; name: "draft/persistence"; aliases: []; request: Request::Default("draft/persistence"); features: [Feature::Persistence]; code: [crate::persistence, crate::state::persistence];
    CHANNEL_RENAME = "draft/channel-rename"; "Channel renaming."; name: "draft/channel-rename"; aliases: []; request: Request::Default("draft/channel-rename"); features: [Feature::ChannelRename]; code: [crate::channel_rename, crate::state::rename];
    #[cfg(feature = "unstable")] RELAYMSG = "draft/relaymsg"; "Relayed messages."; name: "draft/relaymsg"; aliases: []; request: Request::Default("draft/relaymsg"); features: [Feature::Relay]; code: [crate::relaymsg];
    MESSAGE_REDACTION = "draft/message-redaction"; "Message redaction."; name: "draft/message-redaction"; aliases: []; request: Request::Default("draft/message-redaction"); features: [Feature::Redaction]; code: [crate::outgoing, crate::replies];
    #[cfg(feature = "unstable")] WEBPUSH = "draft/webpush"; "Web Push subscriptions."; name: "draft/webpush"; aliases: ["soju.im/webpush"]; request: Request::Default("draft/webpush"); features: [Feature::WebPush]; code: [crate::webpush];
    PRE_AWAY = "draft/pre-away"; "Away state during registration."; name: "draft/pre-away"; aliases: []; request: Request::Default("draft/pre-away"); features: []; code: [crate::state];
    NO_IMPLICIT_NAMES = "draft/no-implicit-names"; "Suppression of implicit names replies."; name: "no-implicit-names"; aliases: ["draft/no-implicit-names"]; request: Request::Default("draft/no-implicit-names"); features: []; code: [crate::state::membership];
    WHOAMI = "draft/whoami"; "Current connection identity discovery."; name: "draft/whoami"; aliases: []; request: Request::Default("draft/whoami"); features: []; code: [crate::state];
    EXTENDED_ISUPPORT = "draft/extended-isupport"; "Extended ISUPPORT discovery."; name: "draft/extended-isupport"; aliases: []; request: Request::Default("draft/extended-isupport"); features: []; code: [crate::isupport];
}

const fn defaults_len() -> usize {
    let (mut index, mut count) = (0, 0);
    while index < ALL.len() {
        if matches!(ALL[index].request, Request::Default(_)) {
            count += 1;
        }
        index += 1;
    }
    count
}

const fn defaults() -> [&'static str; defaults_len()] {
    let (mut source, mut destination) = (0, 0);
    let mut result = [""; defaults_len()];
    while source < ALL.len() {
        if let Request::Default(name) = ALL[source].request {
            result[destination] = name;
            destination += 1;
        }
        source += 1;
    }
    result
}

/// Capabilities requested by a default session when advertised.
pub const DEFAULTS: &[&str] = &defaults();

const fn same(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

/// Returns the preferred spelling of a known capability.
#[must_use]
pub const fn canonical(name: &str) -> &str {
    let mut index = 0;
    while index < ALL.len() {
        let info = &ALL[index];
        if same(name, info.name) {
            return info.name;
        }
        let mut alias = 0;
        while alias < info.aliases.len() {
            if same(name, info.aliases[alias]) {
                return info.name;
            }
            alias += 1;
        }
        index += 1;
    }
    name
}

/// Reports whether a capability name is in the registry.
#[must_use]
pub const fn is_known(name: &str) -> bool {
    !same(canonical(name), name) || known_primary(name)
}

pub(crate) const fn alias(name: &str, index: usize) -> &'static str {
    let canonical = canonical(name);
    let mut entry = 0;
    while entry < ALL.len() {
        if same(canonical, ALL[entry].name) {
            return ALL[entry].aliases[index];
        }
        entry += 1;
    }
    panic!("capability alias requested for an unknown name")
}

const fn known_primary(name: &str) -> bool {
    let mut index = 0;
    while index < ALL.len() {
        if same(name, ALL[index].name) {
            return true;
        }
        index += 1;
    }
    false
}

/// Finds the preferred spelling of a wanted capability advertised by a server.
#[must_use]
pub fn advertised_spelling<'a>(wanted: &str, advertised: &'a Capabilities) -> Option<&'a str> {
    let canonical = canonical(wanted);
    match ALL.iter().find(|info| info.name == canonical) {
        Some(info) => std::iter::once(info.name)
            .chain(info.aliases.iter().copied())
            .find_map(|name| {
                advertised
                    .get_key_value(&Cap::from(name))
                    .map(|(name, _)| name.as_str())
            }),
        None => advertised
            .get_key_value(&Cap::from(wanted))
            .map(|(name, _)| name.as_str()),
    }
}

/// Renders the capability registry as a Markdown implementation table.
#[must_use]
pub fn markdown_table() -> String {
    use std::fmt::Write as _;

    let mut table = String::from(
        "| Capability | Aliases | Requested | Features | Code |\n|---|---|---|---|---|\n",
    );
    for info in ALL {
        let aliases = info.aliases.join(", ");
        let features = info
            .features
            .iter()
            .map(|feature| feature.name())
            .collect::<Vec<_>>()
            .join(", ");
        let code = info
            .code
            .iter()
            .map(|path| format!("`{}`", path.replace(' ', "").trim_start_matches("crate::")))
            .collect::<Vec<_>>()
            .join(", ");
        let requested = match info.request {
            Request::Default(_) => "default",
            Request::Automatic => "automatic",
            Request::Never => "never",
            Request::Superseded(_) => "fallback",
        };
        writeln!(
            table,
            "| `{}` | {} | {requested} | {} | {code} |",
            info.name,
            if aliases.is_empty() { "—" } else { &aliases },
            if features.is_empty() {
                "—"
            } else {
                &features
            },
        )
        .expect("writing to a string cannot fail");
    }
    table
}

pub(crate) fn requestable(name: &str, advertised: &Capabilities, wanted: &[String]) -> bool {
    let Some(info) = ALL.iter().find(|info| info.name == canonical(name)) else {
        return true;
    };
    if info.name == ECHO_MESSAGE {
        return wanted
            .iter()
            .any(|name| canonical(name) == LABELED_RESPONSE)
            && advertised_spelling(LABELED_RESPONSE, advertised).is_some();
    }
    match info.request {
        Request::Never => false,
        Request::Superseded(preferred) => {
            !wanted
                .iter()
                .any(|name| canonical(name) == canonical(preferred))
                || advertised_spelling(preferred, advertised).is_none()
        }
        Request::Default(_) | Request::Automatic => true,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn aliases_share_identity_and_keep_their_spelling() {
        let draft = Cap::from("draft/chathistory");
        let final_name = Cap::from("chathistory");

        assert_eq!(Cap::from(CHATHISTORY), draft);
        assert_eq!(draft, final_name);
        assert_eq!(draft.to_string(), "draft/chathistory");
        assert_eq!(final_name.to_string(), "chathistory");
        assert_eq!(HashSet::from([draft, final_name]).len(), 1);
    }

    #[test]
    fn resolves_aliases_and_advertised_preferences() {
        assert_eq!(canonical("draft/chathistory"), "chathistory");
        assert_eq!(canonical("draft/metadata-2"), "draft/metadata-3");
        assert!(is_known("account-registration"));
        assert!(!is_known("example.org/unknown"));
        let advertised = Capabilities::from([
            (Cap::from("draft/metadata-2"), None),
            (Cap::from("draft/metadata-3"), None),
        ]);
        assert_eq!(
            advertised_spelling(METADATA, &advertised),
            Some("draft/metadata-3")
        );
    }
}
