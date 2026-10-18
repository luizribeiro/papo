use papo_proto::Numeric;

/// Stable classification of a server refusal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Refusal {
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
    /// An input line exceeded the server limit (`417`).
    InputTooLong,
    /// The server does not recognize the command (`421`).
    UnknownCommand,
    /// The nickname is malformed (`432`).
    ErroneousNickname,
    /// The nickname is already in use (`433`).
    NicknameInUse,
    /// A nickname collided during a network split (`436`).
    NickCollision,
    /// A nickname or channel is temporarily unavailable (`437`).
    TemporarilyUnavailable,
    /// The session is not on the channel (`442`).
    NotOnChannel,
    /// The named user is already on the channel (`443`).
    UserOnChannel,
    /// The supplied server or operator password is incorrect (`464`).
    PasswordIncorrect,
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
    /// The channel requires a registered nickname (`477`).
    RegisteredNicknameRequired,
    /// The action requires IRC operator privileges (`481`).
    NoPrivileges,
    /// The action requires channel operator privileges (`482`).
    ChannelOperatorPrivilegesNeeded,
    /// The channel accepts only secure connections (`489`).
    SecureOnlyChannel,
    /// The host is not authorized for operator access (`491`).
    NoOperHost,
    /// A user mode flag is unknown (`501`).
    UnknownUserMode,
    /// The session tried to change another user's modes (`502`).
    UsersDontMatch,
    /// A channel mode parameter is invalid (`696`).
    InvalidModeParameter,
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
    /// The email address cannot be used for registration.
    UnacceptableEmail,
    /// Connection registration must complete before retrying.
    RegistrationRequired,
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
    /// A refusal without a more specific classification.
    ///
    /// A later minor release may refine a code currently classified as `Other`.
    Other,
}

impl Refusal {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const fn from_numeric(numeric: Numeric) -> Self {
        match numeric.code() {
            401 => Self::NoSuchNick,
            402 => Self::NoSuchServer,
            403 => Self::NoSuchChannel,
            404 => Self::CannotSendToChannel,
            405 => Self::TooManyChannels,
            417 => Self::InputTooLong,
            421 => Self::UnknownCommand,
            432 => Self::ErroneousNickname,
            433 => Self::NicknameInUse,
            436 => Self::NickCollision,
            437 => Self::TemporarilyUnavailable,
            442 => Self::NotOnChannel,
            443 => Self::UserOnChannel,
            464 => Self::PasswordIncorrect,
            471 => Self::ChannelIsFull,
            472 => Self::UnknownMode,
            473 => Self::InviteOnlyChannel,
            474 => Self::BannedFromChannel,
            475 => Self::BadChannelKey,
            476 => Self::BadChannelMask,
            477 => Self::RegisteredNicknameRequired,
            481 => Self::NoPrivileges,
            482 => Self::ChannelOperatorPrivilegesNeeded,
            489 => Self::SecureOnlyChannel,
            491 => Self::NoOperHost,
            501 => Self::UnknownUserMode,
            502 => Self::UsersDontMatch,
            696 => Self::InvalidModeParameter,
            263 => Self::TryAgain,
            _ => Self::Other,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
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
            ("REGISTER" | "VERIFY", "COMPLETE_CONNECTION_REQUIRED") => Self::RegistrationRequired,
            ("VERIFY", "INVALID_CODE") => Self::InvalidVerificationCode,
            ("REGISTER" | "VERIFY", "TEMPORARILY_UNAVAILABLE") => Self::TemporarilyUnavailable,
            ("METADATA", "INVALID_TARGET") => Self::InvalidTarget,
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
            _ => Self::Other,
        }
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
            401 => NoSuchNick, 402 => NoSuchServer, 403 => NoSuchChannel,
            404 => CannotSendToChannel, 405 => TooManyChannels, 417 => InputTooLong,
            421 => UnknownCommand, 432 => ErroneousNickname, 433 => NicknameInUse,
            436 => NickCollision, 437 => TemporarilyUnavailable, 442 => NotOnChannel,
            443 => UserOnChannel, 464 => PasswordIncorrect, 471 => ChannelIsFull,
            472 => UnknownMode, 473 => InviteOnlyChannel, 474 => BannedFromChannel,
            475 => BadChannelKey, 476 => BadChannelMask, 477 => RegisteredNicknameRequired,
            481 => NoPrivileges, 482 => ChannelOperatorPrivilegesNeeded, 489 => SecureOnlyChannel,
            491 => NoOperHost, 501 => UnknownUserMode, 502 => UsersDontMatch,
            696 => InvalidModeParameter, 263 => TryAgain, 999 => Other,
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
            "VERIFY" "COMPLETE_CONNECTION_REQUIRED" => RegistrationRequired,
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
}
