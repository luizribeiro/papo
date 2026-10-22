use papo_proto::Numeric;

#[cfg(feature = "unstable")]
mod unstable;
#[cfg(not(feature = "unstable"))]
mod unstable {
    use super::Refusal;

    pub(super) const fn from_standard(_command: &str, _code: &str) -> Refusal {
        Refusal::Other
    }
}

/// Stable classification of a server refusal.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Refusal {
    /// The server could not process the command (`400`).
    CommandFailed,
    /// The requested nickname is not connected (`401`).
    NoSuchNick,
    /// The requested server is unknown (`402`).
    NoSuchServer,
    /// The requested channel does not exist (`403`).
    NoSuchChannel,
    /// A message could not be sent to the channel (`404`).
    CannotSendToChannel,
    /// The session has joined too many channels (`405`).
    TooManyChannels,
    /// No nickname history exists for the requested nickname (`406`).
    NoNicknameHistory,
    /// A required ping origin or token is missing (`409`).
    MissingOrigin,
    /// A message command did not include a recipient (`411`).
    MissingRecipient,
    /// A message command did not include text (`412`).
    NoTextToSend,
    /// An input line exceeded the server limit (`417`).
    InputTooLong,
    /// The server does not recognize the command (`421`).
    UnknownCommand,
    /// The server's message of the day is unavailable (`422`).
    MotdUnavailable,
    /// The nickname is malformed (`432`).
    ErroneousNickname,
    /// The nickname is already in use (`433`).
    NicknameInUse,
    /// A nickname collided during a network split (`436`).
    NickCollision,
    /// A nickname or channel is temporarily unavailable (`437`).
    TemporarilyUnavailable,
    /// The named user is not on the channel (`441`).
    UserNotInChannel,
    /// The session is not on the channel (`442`).
    NotOnChannel,
    /// The named user is already on the channel (`443`).
    UserOnChannel,
    /// A connection-only detail was sent after connecting (`462`).
    AlreadyConnected,
    /// The supplied server or operator password is incorrect (`464`).
    PasswordIncorrect,
    /// The client is banned from the server (`465`).
    BannedFromServer,
    /// The channel has reached its user limit (`471`).
    ChannelIsFull,
    /// The requested channel mode is unknown (`472`).
    UnknownMode,
    /// The channel is invite-only (`473`).
    InviteOnlyChannel,
    /// The session is banned from the channel (`474`).
    BannedFromChannel,
    /// The supplied channel key is incorrect (`475`).
    BadChannelKey,
    /// A channel mask is malformed (`476`).
    BadChannelMask,
    /// A channel's ban or exception list is full (`478`).
    BanListFull,
    /// The action requires IRC operator privileges (`481`).
    NoPrivileges,
    /// The action requires channel operator privileges (`482`).
    ChannelOperatorPrivilegesNeeded,
    /// A server cannot be disconnected with `KILL` (`483`).
    CannotKillServer,
    /// The target is protected from being kicked (`484`).
    CannotKickProtected,
    /// The channel accepts only secure connections (`489`).
    SecureOnlyChannel,
    /// The host is not authorized for operator access (`491`).
    NoOperHost,
    /// A user mode flag is unknown (`501`).
    UnknownUserMode,
    /// The session tried to change another user's modes (`502`).
    UsersDontMatch,
    /// The server has no help for the requested topic (`524`).
    NoHelpAvailable,
    /// A channel key is malformed (`525`).
    InvalidKey,
    /// The server could not upgrade the connection to TLS (`691`).
    TlsUpgradeFailed,
    /// A channel mode parameter is invalid (`696`).
    InvalidModeParameter,
    /// The list mode entry is already set (`697`).
    ListModeAlreadySet,
    /// The list mode entry is not set (`698`).
    ListModeNotSet,
    /// The IRC operator lacks a required server privilege (`723`).
    InsufficientPrivileges,
    /// The account is locked or otherwise unavailable (`902`).
    AccountLocked,
    /// SASL authentication failed (`904`).
    SaslFailed,
    /// A SASL message exceeded the server's limit (`905`).
    SaslMessageTooLong,
    /// SASL authentication was aborted (`906`).
    SaslAborted,
    /// The server is temporarily too busy (`263`).
    TryAgain,
    /// The requested account name is already registered.
    AccountExists,
    /// The requested account name is invalid or restricted.
    BadAccountName,
    /// The account name must match the current nickname.
    AccountNameMustMatchNick,
    /// A nickname must be selected before continuing.
    NicknameRequired,
    /// The session is already authenticated.
    AlreadyAuthenticated,
    /// The password does not meet the server's strength policy.
    WeakPassword,
    /// The password is otherwise unacceptable.
    UnacceptablePassword,
    /// The server cannot send mail to the address.
    InvalidEmail,
    /// The email address cannot be used for account creation.
    UnacceptableEmail,
    /// The connection must complete before retrying (`451`).
    NotConnectedYet,
    /// The verification code is invalid or expired.
    InvalidVerificationCode,
    /// The target is invalid for the requested operation.
    InvalidTarget,
    /// A metadata key does not follow the required syntax.
    InvalidMetadataKey,
    /// The session cannot modify the metadata key.
    MetadataPermissionDenied,
    /// The requested metadata key is not set.
    MetadataKeyNotSet,
    /// A server metadata limit was reached.
    MetadataLimitReached,
    /// The request is temporarily rate limited.
    RateLimited,
    /// The metadata subscription limit was reached.
    TooManyMetadataSubscriptions,
    /// The requested subcommand is not recognized.
    InvalidSubcommand,
    /// The supplied metadata value is invalid.
    InvalidMetadataValue,
    /// The destination channel name is already in use.
    ChannelNameInUse,
    /// The server does not permit the channel to be renamed.
    ChannelRenameForbidden,
    /// Authentication is required for the requested operation, including numeric `477`.
    AccountRequired,
    /// One or more request parameters are invalid.
    InvalidParameters,
    /// A history message could not be retrieved.
    MessageError,
    /// The requested message-reference type is unavailable.
    InvalidMessageReferenceType,
    /// A channel was renamed while it was being joined.
    ChannelRenamed,
    /// The session is not permitted to redact the message.
    RedactionForbidden,
    /// The message is too old to redact.
    RedactionWindowExpired,
    /// The requested message identifier is unknown.
    UnknownMessage,
    /// A multiline message exceeds the server's byte limit.
    MultilineTooLarge,
    /// A multiline message exceeds the server's line limit.
    MultilineTooManyLines,
    /// Multiline batch members do not share one target.
    MultilineTargetMismatch,
    /// A multiline batch is malformed.
    InvalidMultiline,
    /// A client batch reference tag is invalid.
    InvalidBatchReference,
    /// A client batch was not completed in time.
    BatchTimedOut,
    /// The client batch type is unknown.
    UnknownBatchType,
    /// The requested real name is invalid.
    InvalidRealname,
    /// The server does not permit the real name to be changed.
    RealnameChangeForbidden,
    /// Required parameters are missing.
    MissingParameters,
    /// The server could not complete the operation.
    InternalError,
    /// The request contains invalid UTF-8.
    InvalidUtf8,
    /// A bouncer network identifier does not exist.
    #[cfg(feature = "unstable")]
    InvalidNetwork,
    /// A bouncer network attribute value is invalid.
    #[cfg(feature = "unstable")]
    InvalidAttribute,
    /// A bouncer network attribute cannot be changed.
    #[cfg(feature = "unstable")]
    ReadOnlyAttribute,
    /// A bouncer does not recognize a network attribute.
    #[cfg(feature = "unstable")]
    UnknownAttribute,
    /// A required bouncer network attribute is missing.
    #[cfg(feature = "unstable")]
    MissingAttribute,
    /// Relay messaging is disabled by server configuration.
    #[cfg(feature = "unstable")]
    RelayDisabled,
    /// The session lacks permission to relay a message.
    #[cfg(feature = "unstable")]
    RelayPrivilegesNeeded,
    /// The supplied relay message is blank.
    #[cfg(feature = "unstable")]
    BlankRelayMessage,
    /// The supplied relay nickname is invalid.
    #[cfg(feature = "unstable")]
    InvalidRelayNickname,
    /// The relayed sender is banned from the channel.
    #[cfg(feature = "unstable")]
    BannedRelaySender,
    /// Web Push parameters are invalid.
    #[cfg(feature = "unstable")]
    InvalidWebPushParameters,
    /// The account has reached its Web Push registration limit.
    #[cfg(feature = "unstable")]
    WebPushRegistrationLimit,
    /// Persistence parameters are invalid.
    #[cfg(feature = "unstable")]
    InvalidPersistenceParameters,
    /// A refusal without a more specific classification.
    ///
    /// A later minor release may refine a code currently classified as `Other`.
    Other,
}

impl Refusal {
    pub(crate) const fn from_numeric(numeric: Numeric) -> Self {
        match numeric.code() {
            400 => Self::CommandFailed,
            401 => Self::NoSuchNick,
            402 => Self::NoSuchServer,
            403 => Self::NoSuchChannel,
            404 => Self::CannotSendToChannel,
            405 => Self::TooManyChannels,
            406 => Self::NoNicknameHistory,
            409 => Self::MissingOrigin,
            411 => Self::MissingRecipient,
            412 => Self::NoTextToSend,
            417 => Self::InputTooLong,
            421 => Self::UnknownCommand,
            422 => Self::MotdUnavailable,
            431 => Self::NicknameRequired,
            432 => Self::ErroneousNickname,
            433 => Self::NicknameInUse,
            436 => Self::NickCollision,
            437 => Self::TemporarilyUnavailable,
            441 => Self::UserNotInChannel,
            442 => Self::NotOnChannel,
            443 => Self::UserOnChannel,
            451 => Self::NotConnectedYet,
            461 => Self::MissingParameters,
            462 => Self::AlreadyConnected,
            464 => Self::PasswordIncorrect,
            465 => Self::BannedFromServer,
            471 => Self::ChannelIsFull,
            472 => Self::UnknownMode,
            473 => Self::InviteOnlyChannel,
            474 => Self::BannedFromChannel,
            475 => Self::BadChannelKey,
            476 => Self::BadChannelMask,
            477 => Self::AccountRequired,
            478 => Self::BanListFull,
            481 => Self::NoPrivileges,
            482 => Self::ChannelOperatorPrivilegesNeeded,
            483 => Self::CannotKillServer,
            484 => Self::CannotKickProtected,
            489 => Self::SecureOnlyChannel,
            491 => Self::NoOperHost,
            501 => Self::UnknownUserMode,
            502 => Self::UsersDontMatch,
            524 => Self::NoHelpAvailable,
            525 => Self::InvalidKey,
            691 => Self::TlsUpgradeFailed,
            696 => Self::InvalidModeParameter,
            697 => Self::ListModeAlreadySet,
            698 => Self::ListModeNotSet,
            723 => Self::InsufficientPrivileges,
            902 => Self::AccountLocked,
            904 => Self::SaslFailed,
            905 => Self::SaslMessageTooLong,
            906 => Self::SaslAborted,
            907 => Self::AlreadyAuthenticated,
            263 => Self::TryAgain,
            _ => Self::Other,
        }
    }

    pub(crate) fn from_standard(command: &str, code: &str) -> Self {
        let command = command.to_ascii_uppercase();
        let code = code.to_ascii_uppercase();
        match (command.as_str(), code.as_str()) {
            ("REGISTER", "ACCOUNT_EXISTS") => Self::AccountExists,
            ("REGISTER", "BAD_ACCOUNT_NAME") => Self::BadAccountName,
            ("REGISTER", "ACCOUNT_NAME_MUST_BE_NICK" | "ACCOUNT_NAME_MUST_MATCH_NICK") => {
                Self::AccountNameMustMatchNick
            }
            ("REGISTER", "NEED_NICK") => Self::NicknameRequired,
            ("REGISTER" | "VERIFY", "ALREADY_AUTHENTICATED") => Self::AlreadyAuthenticated,
            ("REGISTER", "WEAK_PASSWORD") => Self::WeakPassword,
            ("REGISTER", "UNACCEPTABLE_PASSWORD") => Self::UnacceptablePassword,
            ("REGISTER", "INVALID_EMAIL") => Self::InvalidEmail,
            ("REGISTER", "UNACCEPTABLE_EMAIL") => Self::UnacceptableEmail,
            ("REGISTER" | "VERIFY", "COMPLETE_CONNECTION_REQUIRED") => Self::NotConnectedYet,
            ("VERIFY", "INVALID_CODE") => Self::InvalidVerificationCode,
            ("REGISTER" | "VERIFY", "TEMPORARILY_UNAVAILABLE") => Self::TemporarilyUnavailable,
            ("METADATA" | "CHATHISTORY" | "REDACT", "INVALID_TARGET") => Self::InvalidTarget,
            ("METADATA", "KEY_INVALID") => Self::InvalidMetadataKey,
            ("METADATA", "KEY_NO_PERMISSION") => Self::MetadataPermissionDenied,
            ("METADATA", "KEY_NOT_SET") => Self::MetadataKeyNotSet,
            ("METADATA", "LIMIT_REACHED") => Self::MetadataLimitReached,
            ("METADATA", "RATE_LIMITED") => Self::RateLimited,
            ("METADATA", "TOO_MANY_SUBS") => Self::TooManyMetadataSubscriptions,
            ("METADATA", "SUBCOMMAND_INVALID") => Self::InvalidSubcommand,
            ("METADATA", "VALUE_INVALID") => Self::InvalidMetadataValue,
            ("RENAME", "CHANNEL_NAME_IN_USE") => Self::ChannelNameInUse,
            ("RENAME", "CANNOT_RENAME") => Self::ChannelRenameForbidden,
            ("*", "ACCOUNT_REQUIRED") => Self::AccountRequired,
            ("CHATHISTORY" | "MARKREAD", "INVALID_PARAMS" | "INVALID_PARAMETERS") => {
                Self::InvalidParameters
            }
            ("CHATHISTORY", "MESSAGE_ERROR") => Self::MessageError,
            ("CHATHISTORY", "INVALID_MSGREFTYPE") => Self::InvalidMessageReferenceType,
            ("JOIN", "CHANNEL_RENAMED") => Self::ChannelRenamed,
            ("REDACT", "REDACT_FORBIDDEN") => Self::RedactionForbidden,
            ("REDACT", "REDACT_WINDOW_EXPIRED") => Self::RedactionWindowExpired,
            ("REDACT", "UNKNOWN_MSGID" | "UNKNOWN_MESSAGE_ID") => Self::UnknownMessage,
            ("BATCH", "MULTILINE_MAX_BYTES") => Self::MultilineTooLarge,
            ("BATCH", "MULTILINE_MAX_LINES") => Self::MultilineTooManyLines,
            ("BATCH", "MULTILINE_INVALID_TARGET") => Self::MultilineTargetMismatch,
            ("BATCH", "MULTILINE_INVALID") => Self::InvalidMultiline,
            ("BATCH", "INVALID_REFTAG") => Self::InvalidBatchReference,
            ("BATCH", "TIMEOUT") => Self::BatchTimedOut,
            ("BATCH", "UNKNOWN_TYPE") => Self::UnknownBatchType,
            ("SETNAME", "INVALID_REALNAME") => Self::InvalidRealname,
            ("SETNAME", "CANNOT_CHANGE_REALNAME") => Self::RealnameChangeForbidden,
            ("MARKREAD", "NEED_MORE_PARAMS") => Self::MissingParameters,
            ("MARKREAD", "INTERNAL_ERROR") => Self::InternalError,
            (_, "INVALID_UTF8") => Self::InvalidUtf8,
            _ => unstable::from_standard(command.as_str(), code.as_str()),
        }
    }

    #[cfg(feature = "unstable")]
    pub(crate) fn from_standard_enabled(
        command: &str,
        code: &str,
        enabled: &std::collections::BTreeSet<crate::Feature>,
    ) -> Self {
        unstable::from_standard_enabled(command, code, enabled)
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the IRC server refused the request")
    }
}

#[cfg(test)]
mod tests {
    use {super::Refusal, papo_proto::Numeric};

    #[test]
    fn classifies_every_numeric_refusal() {
        macro_rules! cases {
            ($($code:literal => $variant:ident),* $(,)?) => {$(
                assert_eq!(Refusal::from_numeric(Numeric::new($code).unwrap()), Refusal::$variant);
            )*};
        }
        cases!(
            400 => CommandFailed, 401 => NoSuchNick, 402 => NoSuchServer, 403 => NoSuchChannel,
            404 => CannotSendToChannel, 405 => TooManyChannels, 412 => NoTextToSend,
            406 => NoNicknameHistory, 409 => MissingOrigin, 411 => MissingRecipient,
            417 => InputTooLong, 421 => UnknownCommand, 422 => MotdUnavailable,
            431 => NicknameRequired, 432 => ErroneousNickname, 433 => NicknameInUse,
            436 => NickCollision, 437 => TemporarilyUnavailable, 441 => UserNotInChannel,
            442 => NotOnChannel,
            443 => UserOnChannel, 451 => NotConnectedYet, 461 => MissingParameters,
            462 => AlreadyConnected, 464 => PasswordIncorrect, 465 => BannedFromServer,
            471 => ChannelIsFull,
            472 => UnknownMode, 473 => InviteOnlyChannel, 474 => BannedFromChannel,
            475 => BadChannelKey, 476 => BadChannelMask, 477 => AccountRequired,
            478 => BanListFull, 481 => NoPrivileges, 482 => ChannelOperatorPrivilegesNeeded,
            483 => CannotKillServer, 484 => CannotKickProtected, 489 => SecureOnlyChannel,
            491 => NoOperHost, 501 => UnknownUserMode,
            502 => UsersDontMatch, 524 => NoHelpAvailable, 525 => InvalidKey,
            691 => TlsUpgradeFailed, 696 => InvalidModeParameter, 697 => ListModeAlreadySet,
            698 => ListModeNotSet,
            723 => InsufficientPrivileges, 902 => AccountLocked, 904 => SaslFailed,
            905 => SaslMessageTooLong, 906 => SaslAborted, 907 => AlreadyAuthenticated,
            263 => TryAgain, 999 => Other,
        );
    }

    #[test]
    fn displays_a_stable_server_refusal_message() {
        assert_eq!(
            Refusal::ChannelOperatorPrivilegesNeeded.to_string(),
            "the IRC server refused the request"
        );
    }

    #[test]
    fn classifies_account_metadata_and_rename_failures() {
        macro_rules! cases {
            ($($command:literal $code:literal => $variant:ident),* $(,)?) => {$(
                assert_eq!(Refusal::from_standard($command, $code), Refusal::$variant);
            )*};
        }
        cases!(
            "REGISTER" "ACCOUNT_EXISTS" => AccountExists,
            "REGISTER" "BAD_ACCOUNT_NAME" => BadAccountName,
            "REGISTER" "ACCOUNT_NAME_MUST_BE_NICK" => AccountNameMustMatchNick,
            "REGISTER" "NEED_NICK" => NicknameRequired,
            "VERIFY" "ALREADY_AUTHENTICATED" => AlreadyAuthenticated,
            "REGISTER" "WEAK_PASSWORD" => WeakPassword,
            "REGISTER" "UNACCEPTABLE_PASSWORD" => UnacceptablePassword,
            "REGISTER" "INVALID_EMAIL" => InvalidEmail,
            "REGISTER" "UNACCEPTABLE_EMAIL" => UnacceptableEmail,
            "VERIFY" "COMPLETE_CONNECTION_REQUIRED" => NotConnectedYet,
            "VERIFY" "INVALID_CODE" => InvalidVerificationCode,
            "REGISTER" "TEMPORARILY_UNAVAILABLE" => TemporarilyUnavailable,
            "METADATA" "INVALID_TARGET" => InvalidTarget,
            "METADATA" "KEY_INVALID" => InvalidMetadataKey,
            "METADATA" "KEY_NO_PERMISSION" => MetadataPermissionDenied,
            "METADATA" "KEY_NOT_SET" => MetadataKeyNotSet,
            "METADATA" "LIMIT_REACHED" => MetadataLimitReached,
            "METADATA" "RATE_LIMITED" => RateLimited,
            "METADATA" "TOO_MANY_SUBS" => TooManyMetadataSubscriptions,
            "METADATA" "SUBCOMMAND_INVALID" => InvalidSubcommand,
            "METADATA" "VALUE_INVALID" => InvalidMetadataValue,
            "RENAME" "CHANNEL_NAME_IN_USE" => ChannelNameInUse,
            "RENAME" "CANNOT_RENAME" => ChannelRenameForbidden,
        );
        assert_eq!(
            Refusal::from_standard("WHOIS", "INVALID_TARGET"),
            Refusal::Other
        );
    }

    #[test]
    fn classifies_message_and_batch_failures() {
        macro_rules! cases {
            ($($command:literal $code:literal => $variant:ident),* $(,)?) => {$(
                assert_eq!(Refusal::from_standard($command, $code), Refusal::$variant);
            )*};
        }
        cases!(
            "*" "ACCOUNT_REQUIRED" => AccountRequired,
            "CHATHISTORY" "INVALID_PARAMS" => InvalidParameters,
            "CHATHISTORY" "INVALID_TARGET" => InvalidTarget,
            "CHATHISTORY" "MESSAGE_ERROR" => MessageError,
            "CHATHISTORY" "INVALID_MSGREFTYPE" => InvalidMessageReferenceType,
            "JOIN" "CHANNEL_RENAMED" => ChannelRenamed,
            "REDACT" "REDACT_FORBIDDEN" => RedactionForbidden,
            "REDACT" "REDACT_WINDOW_EXPIRED" => RedactionWindowExpired,
            "REDACT" "UNKNOWN_MSGID" => UnknownMessage,
            "BATCH" "MULTILINE_MAX_BYTES" => MultilineTooLarge,
            "BATCH" "MULTILINE_MAX_LINES" => MultilineTooManyLines,
            "BATCH" "MULTILINE_INVALID_TARGET" => MultilineTargetMismatch,
            "BATCH" "MULTILINE_INVALID" => InvalidMultiline,
            "BATCH" "INVALID_REFTAG" => InvalidBatchReference,
            "BATCH" "TIMEOUT" => BatchTimedOut,
            "BATCH" "UNKNOWN_TYPE" => UnknownBatchType,
            "SETNAME" "INVALID_REALNAME" => InvalidRealname,
            "SETNAME" "CANNOT_CHANGE_REALNAME" => RealnameChangeForbidden,
            "MARKREAD" "NEED_MORE_PARAMS" => MissingParameters,
            "MARKREAD" "INTERNAL_ERROR" => InternalError,
            "PRIVMSG" "INVALID_UTF8" => InvalidUtf8,
        );
    }

    #[cfg(feature = "unstable")]
    #[test]
    fn classifies_unstable_extension_failures() {
        macro_rules! cases {
            ($($command:literal $code:literal => $variant:ident),* $(,)?) => {$(
                assert_eq!(Refusal::from_standard($command, $code), Refusal::$variant);
            )*};
        }
        cases!(
            "BOUNCER" "INVALID_NETID" => InvalidNetwork,
            "BOUNCER" "INVALID_ATTRIBUTE" => InvalidAttribute,
            "BOUNCER" "READ_ONLY_ATTRIBUTE" => ReadOnlyAttribute,
            "BOUNCER" "UNKNOWN_ATTRIBUTE" => UnknownAttribute,
            "BOUNCER" "NEED_ATTRIBUTE" => MissingAttribute,
            "RELAYMSG" "NOT_ENABLED" => RelayDisabled,
            "RELAYMSG" "PRIVS_NEEDED" => RelayPrivilegesNeeded,
            "RELAYMSG" "BLANK_MSG" => BlankRelayMessage,
            "RELAYMSG" "INVALID_NICK" => InvalidRelayNickname,
            "RELAYMSG" "BANNED" => BannedRelaySender,
            "WEBPUSH" "INVALID_PARAMS" => InvalidWebPushParameters,
            "WEBPUSH" "MAX_REGISTRATIONS" => WebPushRegistrationLimit,
            "PERSISTENCE" "ACCOUNT_REQUIRED" => AccountRequired,
            "PERSISTENCE" "INVALID_PARAMETERS" => InvalidPersistenceParameters,
            "PERSISTENCE" "INTERNAL_ERROR" => InternalError,
        );
    }

    #[cfg(feature = "unstable")]
    #[test]
    fn gates_unstable_extension_failures_by_enabled_feature() {
        use std::collections::BTreeSet;

        use crate::Feature;

        for (feature, command, code, expected) in [
            (
                Feature::Bouncer,
                "BOUNCER",
                "INVALID_NETID",
                Refusal::InvalidNetwork,
            ),
            (
                Feature::Bouncer,
                "BOUNCER",
                "INVALID_ATTRIBUTE",
                Refusal::InvalidAttribute,
            ),
            (
                Feature::Bouncer,
                "BOUNCER",
                "READ_ONLY_ATTRIBUTE",
                Refusal::ReadOnlyAttribute,
            ),
            (
                Feature::Bouncer,
                "BOUNCER",
                "UNKNOWN_ATTRIBUTE",
                Refusal::UnknownAttribute,
            ),
            (
                Feature::Bouncer,
                "BOUNCER",
                "NEED_ATTRIBUTE",
                Refusal::MissingAttribute,
            ),
            (
                Feature::WebPush,
                "WEBPUSH",
                "INVALID_PARAMS",
                Refusal::InvalidWebPushParameters,
            ),
            (
                Feature::WebPush,
                "WEBPUSH",
                "MAX_REGISTRATIONS",
                Refusal::WebPushRegistrationLimit,
            ),
            (
                Feature::Persistence,
                "PERSISTENCE",
                "INVALID_PARAMETERS",
                Refusal::InvalidPersistenceParameters,
            ),
        ] {
            assert_eq!(
                Refusal::from_standard_enabled(command, code, &BTreeSet::new()),
                Refusal::Other,
                "{command} {code} without {feature:?}"
            );
            assert_eq!(
                Refusal::from_standard_enabled(command, code, &BTreeSet::from([feature])),
                expected,
                "{command} {code} with {feature:?}"
            );
        }

        for (command, code, expected) in [
            ("WEBPUSH", "INTERNAL_ERROR", Refusal::InternalError),
            ("PERSISTENCE", "ACCOUNT_REQUIRED", Refusal::AccountRequired),
            ("PERSISTENCE", "INTERNAL_ERROR", Refusal::InternalError),
        ] {
            assert_eq!(
                Refusal::from_standard_enabled(command, code, &BTreeSet::new()),
                expected,
                "{command} {code}"
            );
        }
    }
}
