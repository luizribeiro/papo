//! IRC capability names and their implementation homes.

use std::{
    borrow::{Borrow, Cow},
    cmp::Ordering,
    hash::{Hash, Hasher},
};

use crate::{Capabilities, Config, Feature};

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

    pub(crate) const fn constant_str(&self) -> &'static str {
        match &self.0 {
            Cow::Borrowed(name) => name,
            Cow::Owned(_) => panic!("owned capability has no static spelling"),
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

impl From<&String> for Cap {
    fn from(name: &String) -> Self {
        Self::from(name.as_str())
    }
}

impl From<Cap> for String {
    fn from(name: Cap) -> Self {
        name.0.into_owned()
    }
}

impl AsRef<str> for Cap {
    fn as_ref(&self) -> &str {
        self.as_str()
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
    /// Requested only when the application enables the corresponding feature.
    #[cfg(feature = "unstable")]
    OptIn(&'static str, Feature),
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

macro_rules! capability_constant {
    (#[cfg(feature = $feature:literal)] [cap] $constant:ident = $value:literal; $doc:literal) => {
        #[cfg(feature = $feature)]
        #[doc = $doc]
        pub const $constant: Cap = Cap::new($value);
    };
    (#[cfg(feature = $feature:literal)] $constant:ident = $value:literal; $doc:literal) => {
        #[cfg(feature = $feature)]
        #[doc = $doc]
        pub const $constant: &str = $value;
    };
    ($(#[$meta:meta])* [cap] $constant:ident = $value:literal; $doc:literal) => {
        $(#[$meta])* #[doc = $doc] pub const $constant: Cap = Cap::new($value);
    };
    ($(#[$meta:meta])* $constant:ident = $value:literal; $doc:literal) => {
        $(#[$meta])* #[doc = $doc] pub const $constant: &str = $value;
    };
}

macro_rules! capabilities {
    ($($(#[$meta:meta])* $([$kind:ident])? $constant:ident = $value:literal; $doc:literal;
        name: $name:literal; aliases: [$($alias:literal),*]; request: $request:expr;
        features: [$($feature:path),*]; code: [$($code:path),+];)*) => {
        $(capability_constant! {
            $(#[$meta])* $([$kind])? $constant = $value; $doc
        })*

        /// Every capability understood by papo.
        pub static ALL: &[CapInfo] = &[$($(#[$meta])* CapInfo {
            name: $name, aliases: &[$($alias),*], request: $request,
            features: &[$($feature),*], code: &[$(stringify!($code)),+],
        },)*];

        $($(#[$meta])* const _: () = { $(#[allow(unused_imports)] use $code as _;)+ };)*
    };
}

capabilities! {
    [cap] CAP_NOTIFY = "cap-notify"; "Capability change notifications."; name: "cap-notify"; aliases: []; request: Request::Default("cap-notify"); features: [Feature::CapabilityNotifications]; code: [crate::caps];
    [cap] MESSAGE_TAGS = "message-tags"; "`IRCv3` message tags."; name: "message-tags"; aliases: []; request: Request::Default("message-tags"); features: [Feature::MessageTags, Feature::Replies, Feature::Reactions, Feature::Typing]; code: [crate::features, crate::replies];
    [cap] SERVER_TIME = "server-time"; "Server-assigned message timestamps."; name: "server-time"; aliases: []; request: Request::Default("server-time"); features: [Feature::ServerTime]; code: [crate::state::read_marker];
    [cap] BATCH = "batch"; "Batched messages."; name: "batch"; aliases: []; request: Request::Default("batch"); features: [Feature::Batches]; code: [crate::batch];
    [cap] EVENT_PLAYBACK = "draft/event-playback"; "Replay of missed events."; name: "draft/event-playback"; aliases: []; request: Request::Default("draft/event-playback"); features: [Feature::EventPlayback]; code: [crate::batch];
    [cap] ECHO_MESSAGE = "echo-message"; "Echoes of sent messages."; name: "echo-message"; aliases: []; request: Request::Default("echo-message"); features: [Feature::EchoMessages]; code: [crate::outgoing, crate::echo];
    [cap] ACCOUNT_TAG = "account-tag"; "Account tags on messages."; name: "account-tag"; aliases: []; request: Request::Default("account-tag"); features: [Feature::AccountTracking]; code: [crate::state::presence];
    [cap] EXTENDED_JOIN = "extended-join"; "Account and real-name fields on joins."; name: "extended-join"; aliases: []; request: Request::Default("extended-join"); features: [Feature::ExtendedJoin]; code: [crate::state::membership];
    [cap] MULTI_PREFIX = "multi-prefix"; "Every membership prefix in names replies."; name: "multi-prefix"; aliases: []; request: Request::Default("multi-prefix"); features: [Feature::MultiPrefix]; code: [crate::state::membership];
    [cap] USERHOST_IN_NAMES = "userhost-in-names"; "User and host fields in names replies."; name: "userhost-in-names"; aliases: []; request: Request::Default("userhost-in-names"); features: [Feature::UserhostInNames]; code: [crate::state::membership];
    [cap] CHGHOST = "chghost"; "Host-change notifications."; name: "chghost"; aliases: []; request: Request::Default("chghost"); features: [Feature::HostTracking]; code: [crate::state::presence];
    [cap] SETNAME = "setname"; "Real-name changes."; name: "setname"; aliases: []; request: Request::Default("setname"); features: [Feature::SetName]; code: [crate::state::presence];
    [cap] AWAY_NOTIFY = "away-notify"; "Away-state notifications."; name: "away-notify"; aliases: []; request: Request::Default("away-notify"); features: [Feature::AwayTracking]; code: [crate::state::presence];
    [cap] ACCOUNT_NOTIFY = "account-notify"; "Account-change notifications."; name: "account-notify"; aliases: []; request: Request::Default("account-notify"); features: [Feature::AccountTracking]; code: [crate::state::presence];
    [cap] EXTENDED_MONITOR = "extended-monitor"; "Extended monitor replies."; name: "extended-monitor"; aliases: []; request: Request::Default("extended-monitor"); features: [Feature::ExtendedMonitor]; code: [crate::monitor];
    [cap] INVITE_NOTIFY = "invite-notify"; "Invitation notifications."; name: "invite-notify"; aliases: []; request: Request::Default("invite-notify"); features: [Feature::InviteTracking]; code: [crate::replies];
    [cap] STANDARD_REPLIES = "standard-replies"; "Structured standard replies."; name: "standard-replies"; aliases: []; request: Request::Default("standard-replies"); features: [Feature::StandardReplies]; code: [crate::refusal, crate::replies];
    [cap] LABELED_RESPONSE = "labeled-response"; "Labelled request correlation."; name: "labeled-response"; aliases: []; request: Request::Default("labeled-response"); features: [Feature::LabeledResponse]; code: [crate::outgoing, crate::labels];
    [cap] MULTILINE = "draft/multiline"; "Multiline messages."; name: "draft/multiline"; aliases: []; request: Request::Default("draft/multiline"); features: [Feature::Multiline]; code: [crate::outgoing, crate::batch];
    [cap] OPER_TAG = "draft/oper-tag"; "Operator tags on messages."; name: "draft/oper-tag"; aliases: []; request: Request::Default("draft/oper-tag"); features: [Feature::OperatorTags]; code: [crate::replies];
    [cap] METADATA = "draft/metadata-2"; "Client metadata."; name: "draft/metadata-3"; aliases: ["draft/metadata-2"]; request: Request::Default("draft/metadata-2"); features: [Feature::Metadata]; code: [crate::metadata, crate::state::metadata];
    [cap] METADATA_NOTIFY = "metadata-notify"; "Legacy metadata notifications."; name: "metadata-notify"; aliases: []; request: Request::Superseded("draft/metadata-3"); features: []; code: [crate::metadata, crate::state::metadata];
    [cap] SASL = "sasl"; "SASL authentication."; name: "sasl"; aliases: []; request: Request::Automatic; features: [Feature::Sasl]; code: [crate::sasl];
    [cap] STS = "sts"; "Strict Transport Security policy."; name: "sts"; aliases: []; request: Request::Never; features: []; code: [crate::sts];
    [cap] NOPE = "ergo.chat/nope"; "Ergo capability-negotiation test trap."; name: "ergo.chat/nope"; aliases: []; request: Request::Never; features: []; code: [crate::caps];
    [cap] ACCOUNT_REQUIRED = "soju.im/account-required"; "Soju authentication requirement."; name: "soju.im/account-required"; aliases: []; request: Request::Never; features: []; code: [crate::caps];
    [cap] ACCOUNT_REGISTRATION = "draft/account-registration"; "Account registration."; name: "draft/account-registration"; aliases: ["account-registration"]; request: Request::Default("draft/account-registration"); features: [Feature::AccountCreation]; code: [crate::account_registration];
    #[cfg(feature = "unstable")] [cap] BOUNCER_NETWORKS = "soju.im/bouncer-networks"; "Bouncer network management."; name: "soju.im/bouncer-networks"; aliases: []; request: Request::OptIn("soju.im/bouncer-networks", Feature::Bouncer); features: [Feature::Bouncer]; code: [crate::bouncer];
    #[cfg(feature = "unstable")] [cap] BOUNCER_NETWORKS_NOTIFY = "soju.im/bouncer-networks-notify"; "Bouncer network change notifications."; name: "soju.im/bouncer-networks-notify"; aliases: []; request: Request::OptIn("soju.im/bouncer-networks-notify", Feature::Bouncer); features: []; code: [crate::bouncer];
    [cap] CHATHISTORY = "draft/chathistory"; "Message history queries."; name: "draft/chathistory"; aliases: ["chathistory"]; request: Request::Default("draft/chathistory"); features: [Feature::History]; code: [crate::history, crate::batch];
    [cap] READ_MARKER = "draft/read-marker"; "Channel read markers."; name: "draft/read-marker"; aliases: ["read-marker"]; request: Request::Default("draft/read-marker"); features: [Feature::ReadMarkers]; code: [crate::history, crate::state::read_marker];
    #[cfg(feature = "unstable")] [cap] PERSISTENCE = "draft/persistence"; "Account persistence preferences."; name: "draft/persistence"; aliases: []; request: Request::OptIn("draft/persistence", Feature::Persistence); features: [Feature::Persistence]; code: [crate::persistence, crate::state::persistence];
    [cap] CHANNEL_RENAME = "draft/channel-rename"; "Channel renaming."; name: "draft/channel-rename"; aliases: []; request: Request::Default("draft/channel-rename"); features: [Feature::ChannelRename]; code: [crate::channel_rename, crate::state::rename];
    #[cfg(feature = "unstable")] [cap] RELAYMSG = "draft/relaymsg"; "Relayed messages."; name: "draft/relaymsg"; aliases: []; request: Request::OptIn("draft/relaymsg", Feature::Relay); features: [Feature::Relay]; code: [crate::relaymsg];
    [cap] MESSAGE_REDACTION = "draft/message-redaction"; "Message redaction."; name: "draft/message-redaction"; aliases: []; request: Request::Default("draft/message-redaction"); features: [Feature::Redaction]; code: [crate::outgoing, crate::replies];
    #[cfg(feature = "unstable")] [cap] WEBPUSH = "draft/webpush"; "Web Push subscriptions."; name: "draft/webpush"; aliases: ["soju.im/webpush"]; request: Request::OptIn("draft/webpush", Feature::WebPush); features: [Feature::WebPush]; code: [crate::webpush];
    [cap] PRE_AWAY = "draft/pre-away"; "Away state during registration."; name: "draft/pre-away"; aliases: []; request: Request::Default("draft/pre-away"); features: [Feature::PreAway]; code: [crate::state];
    [cap] NO_IMPLICIT_NAMES = "no-implicit-names"; "Suppression of implicit names replies."; name: "no-implicit-names"; aliases: ["draft/no-implicit-names"]; request: Request::Default("no-implicit-names"); features: [Feature::NoImplicitNames]; code: [crate::state::membership];
    [cap] WHOAMI = "draft/whoami"; "Current connection identity discovery."; name: "draft/whoami"; aliases: []; request: Request::Default("draft/whoami"); features: [Feature::WhoAmI]; code: [crate::state];
    [cap] EXTENDED_ISUPPORT = "draft/extended-isupport"; "Extended ISUPPORT discovery."; name: "draft/extended-isupport"; aliases: []; request: Request::Default("draft/extended-isupport"); features: [Feature::ExtendedIsupport]; code: [crate::isupport];
}

pub(crate) const CHATHISTORY_BATCH: &str = "chathistory";
pub(crate) const BOUNCER_NETWORKS_NAME: &str = "soju.im/bouncer-networks";

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

/// Reports whether a capability is known in any build configuration.
#[doc(hidden)]
#[must_use]
pub const fn __is_known(name: &str) -> bool {
    is_known(name) || is_unstable(name)
}

/// Returns the feature that controls a known capability.
#[doc(hidden)]
#[must_use]
pub fn __feature(name: &str) -> Option<Feature> {
    ALL.iter()
        .find(|info| info.name == canonical(name))
        .and_then(|info| info.features.first().copied())
}

/// Reports whether a capability belongs to an opt-in unstable feature.
#[doc(hidden)]
#[must_use]
pub const fn is_unstable(name: &str) -> bool {
    same(name, "soju.im/bouncer-networks")
        || same(name, "soju.im/bouncer-networks-notify")
        || same(name, "draft/persistence")
        || same(name, "draft/relaymsg")
        || same(name, "draft/webpush")
        || same(name, "soju.im/webpush")
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

pub(crate) fn request_spelling(name: &str) -> Option<&'static str> {
    ALL.iter()
        .find(|info| info.name == canonical(name))
        .and_then(|info| match info.request {
            Request::Default(spelling) => Some(spelling),
            #[cfg(feature = "unstable")]
            Request::OptIn(spelling, _) => Some(spelling),
            Request::Automatic | Request::Never | Request::Superseded(_) => None,
        })
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
        Some(info)
            if matches!(info.request, Request::Default(_)) && !accepts_request_aliases(info) =>
        {
            let Request::Default(spelling) = info.request else {
                unreachable!()
            };
            advertised
                .get_key_value(&Cap::from(spelling))
                .filter(|(name, _)| name.as_str() == spelling)
                .map(|(name, _)| name.as_str())
        }
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

fn accepts_request_aliases(info: &CapInfo) -> bool {
    cfg!(feature = "unstable") && info.name == "draft/webpush"
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
            #[cfg(feature = "unstable")]
            Request::OptIn(_, _) => "opt-in",
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

pub(crate) fn requestable(
    name: &str,
    advertised: &Capabilities,
    wanted: &[String],
    config: &Config,
) -> bool {
    _ = config;
    if !cfg!(feature = "unstable") && is_unstable(name) {
        return false;
    }
    let Some(info) = ALL.iter().find(|info| info.name == canonical(name)) else {
        return true;
    };
    if info.name == ECHO_MESSAGE.as_str() {
        return wanted
            .iter()
            .any(|name| canonical(name) == LABELED_RESPONSE.constant_str())
            && advertised_spelling(LABELED_RESPONSE.constant_str(), advertised).is_some();
    }
    match info.request {
        Request::Never => false,
        Request::Superseded(preferred) => {
            !wanted
                .iter()
                .any(|name| canonical(name) == canonical(preferred))
                || advertised_spelling(preferred, advertised).is_none()
        }
        #[cfg(feature = "unstable")]
        Request::OptIn(_, feature) => {
            config.enabled_features.contains(&feature)
                || (info.name == BOUNCER_NETWORKS.as_str()
                    && config.selected_bouncer_network().is_some())
        }
        Request::Default(_) | Request::Automatic => true,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cmp::Ordering,
        collections::{HashSet, hash_map::DefaultHasher},
        hash::{Hash, Hasher},
    };

    use super::*;

    #[test]
    fn aliases_share_identity_and_keep_their_spelling() {
        let draft = Cap::from("draft/chathistory");
        let final_name = Cap::from("chathistory");
        let different = Cap::from("server-time");

        assert_eq!(CHATHISTORY, draft);
        assert_eq!(draft, final_name);
        assert_ne!(draft, different);
        assert_eq!(draft.partial_cmp(&final_name), Some(Ordering::Equal));
        assert_eq!(draft.cmp(&final_name), Ordering::Equal);
        assert_eq!(draft.partial_cmp(&different), Some(draft.cmp(&different)));
        assert_eq!(draft.to_string(), "draft/chathistory");
        assert_eq!(final_name.to_string(), "chathistory");
        assert_eq!(HashSet::from([draft.clone(), final_name.clone()]).len(), 1);

        let hash = |cap: &Cap| {
            let mut hasher = DefaultHasher::new();
            cap.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash(&draft), hash(&final_name));
        assert_ne!(hash(&draft), hash(&different));
        assert_eq!(
            String::from(Cap::from("draft/chathistory")),
            "draft/chathistory"
        );
        assert_eq!(Cap::from("chathistory").as_ref(), "chathistory");
        let borrowed = "server-time".to_owned();
        assert_eq!(Cap::from(&borrowed).as_ref(), "server-time");
    }

    #[test]
    fn resolves_aliases_and_advertised_preferences() {
        assert_eq!(canonical("draft/chathistory"), "draft/chathistory");
        assert_eq!(canonical("chathistory"), "draft/chathistory");
        assert_eq!(canonical("draft/metadata-2"), "draft/metadata-3");
        assert!(is_known("account-registration"));
        assert!(!is_known("example.org/unknown"));
        let advertised = Capabilities::from([(Cap::from("draft/metadata-2"), None)]);
        assert_eq!(
            advertised_spelling(METADATA.as_str(), &advertised),
            Some("draft/metadata-2")
        );
        assert_eq!(
            advertised_spelling(
                CHATHISTORY.as_str(),
                &Capabilities::from([(Cap::from("chathistory"), None)])
            ),
            None
        );
    }

    #[test]
    fn identifies_every_unstable_capability_spelling() {
        for name in [
            "soju.im/bouncer-networks",
            "soju.im/bouncer-networks-notify",
            "draft/persistence",
            "draft/relaymsg",
            "draft/webpush",
            "soju.im/webpush",
        ] {
            assert!(is_unstable(name), "{name}");
        }
        assert!(!is_unstable("server-time"));
    }

    #[test]
    fn requests_published_spellings_from_ergo_cap_ls() {
        for (advertised, requested) in [
            (
                "draft/account-registration",
                Some("draft/account-registration"),
            ),
            ("account-registration", Some("draft/account-registration")),
            ("draft/chathistory", Some("draft/chathistory")),
            ("chathistory", Some("draft/chathistory")),
            ("draft/metadata-2", Some("draft/metadata-2")),
            ("draft/metadata-3", Some("draft/metadata-2")),
            ("no-implicit-names", Some("no-implicit-names")),
            ("draft/no-implicit-names", Some("no-implicit-names")),
            ("sasl", None),
            ("ergo.chat/nope", None),
            ("znc.in/playback", None),
        ] {
            assert_eq!(request_spelling(advertised), requested, "{advertised}");
        }

        for name in [
            "draft/account-registration",
            "draft/chathistory",
            "draft/metadata-3",
            "no-implicit-names",
        ] {
            let info = ALL.iter().find(|info| info.name == name).unwrap();
            assert!(!accepts_request_aliases(info), "{name}");
        }

        #[cfg(feature = "unstable")]
        {
            for name in [
                "draft/persistence",
                "draft/relaymsg",
                "draft/webpush",
                "soju.im/webpush",
            ] {
                assert_eq!(request_spelling(name), Some(canonical(name)), "{name}");
            }
            let webpush = ALL
                .iter()
                .find(|info| info.name == "draft/webpush")
                .unwrap();
            assert!(accepts_request_aliases(webpush));
        }
    }
}
