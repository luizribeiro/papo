use crate::{Feature as F, Refusal as R};

struct Entry {
    command: &'static str,
    code: &'static str,
    refusal: R,
    feature: Option<F>,
}

macro_rules! entry {
    ($command:literal, $code:literal, $refusal:ident, $feature:ident) => {
        Entry {
            command: $command,
            code: $code,
            refusal: R::$refusal,
            feature: Some(F::$feature),
        }
    };
    ($command:literal, $code:literal, $refusal:ident) => {
        Entry {
            command: $command,
            code: $code,
            refusal: R::$refusal,
            feature: None,
        }
    };
}

const STANDARD: &[Entry] = &[
    entry!("BOUNCER", "INVALID_NETID", InvalidNetwork, Bouncer),
    entry!("BOUNCER", "INVALID_ATTRIBUTE", InvalidAttribute, Bouncer),
    entry!("BOUNCER", "READ_ONLY_ATTRIBUTE", ReadOnlyAttribute, Bouncer),
    entry!("BOUNCER", "UNKNOWN_ATTRIBUTE", UnknownAttribute, Bouncer),
    entry!("BOUNCER", "NEED_ATTRIBUTE", MissingAttribute, Bouncer),
    entry!("RELAYMSG", "NOT_ENABLED", RelayDisabled, Relay),
    entry!("RELAYMSG", "PRIVS_NEEDED", RelayPrivilegesNeeded, Relay),
    entry!("RELAYMSG", "BLANK_MSG", BlankRelayMessage, Relay),
    entry!("RELAYMSG", "INVALID_NICK", InvalidRelayNickname, Relay),
    entry!("RELAYMSG", "BANNED", BannedRelaySender, Relay),
    entry!(
        "WEBPUSH",
        "INVALID_PARAMS",
        InvalidWebPushParameters,
        WebPush
    ),
    entry!("WEBPUSH", "INTERNAL_ERROR", InternalError),
    entry!(
        "WEBPUSH",
        "MAX_REGISTRATIONS",
        WebPushRegistrationLimit,
        WebPush
    ),
    entry!("PERSISTENCE", "ACCOUNT_REQUIRED", AccountRequired),
    entry!(
        "PERSISTENCE",
        "INVALID_PARAMETERS",
        InvalidPersistenceParameters,
        Persistence
    ),
    entry!(
        "PERSISTENCE",
        "INVALID_PARAMS",
        InvalidPersistenceParameters,
        Persistence
    ),
    entry!("PERSISTENCE", "INTERNAL_ERROR", InternalError),
];

pub(super) fn from_standard(command: &str, code: &str) -> R {
    lookup(command, code).map_or(R::Other, |(refusal, _)| refusal)
}

fn lookup(command: &str, code: &str) -> Option<(R, Option<F>)> {
    STANDARD
        .iter()
        .find(|entry| entry.command == command && entry.code == code)
        .map(|entry| (entry.refusal, entry.feature))
}
