//! Standard-reply `FAIL` code constants.

macro_rules! codes {
    ($($(#[$attribute:meta])* $name:ident),+ $(,)?) => {
        $(
            $(#[$attribute])*
            #[doc = concat!("The `", stringify!($name), "` failure code.")]
            pub const $name: &str = stringify!($name);
        )+
    };
}

codes!(
    ACCOUNT_EXISTS,
    BAD_ACCOUNT_NAME,
    ACCOUNT_NAME_MUST_BE_NICK,
    ACCOUNT_NAME_MUST_MATCH_NICK,
    NEED_NICK,
    ALREADY_AUTHENTICATED,
    WEAK_PASSWORD,
    UNACCEPTABLE_PASSWORD,
    INVALID_EMAIL,
    UNACCEPTABLE_EMAIL,
    COMPLETE_CONNECTION_REQUIRED,
    INVALID_CODE,
    TEMPORARILY_UNAVAILABLE,
    INVALID_TARGET,
    KEY_INVALID,
    KEY_NO_PERMISSION,
    KEY_NOT_SET,
    LIMIT_REACHED,
    RATE_LIMITED,
    TOO_MANY_SUBS,
    SUBCOMMAND_INVALID,
    VALUE_INVALID,
    CHANNEL_NAME_IN_USE,
    CANNOT_RENAME,
    ACCOUNT_REQUIRED,
    INVALID_PARAMS,
    INVALID_PARAMETERS,
    MESSAGE_ERROR,
    INVALID_MSGREFTYPE,
    CHANNEL_RENAMED,
    REDACT_FORBIDDEN,
    REDACT_WINDOW_EXPIRED,
    UNKNOWN_MSGID,
    UNKNOWN_MESSAGE_ID,
    MULTILINE_MAX_BYTES,
    MULTILINE_MAX_LINES,
    MULTILINE_INVALID_TARGET,
    MULTILINE_INVALID,
    INVALID_REFTAG,
    TIMEOUT,
    UNKNOWN_TYPE,
    INVALID_REALNAME,
    CANNOT_CHANGE_REALNAME,
    NEED_MORE_PARAMS,
    INTERNAL_ERROR,
    INVALID_UTF8,
    #[cfg(feature = "unstable")]
    INVALID_NETID,
    #[cfg(feature = "unstable")]
    INVALID_ATTRIBUTE,
    #[cfg(feature = "unstable")]
    READ_ONLY_ATTRIBUTE,
    #[cfg(feature = "unstable")]
    UNKNOWN_ATTRIBUTE,
    #[cfg(feature = "unstable")]
    NEED_ATTRIBUTE,
    #[cfg(feature = "unstable")]
    NOT_ENABLED,
    #[cfg(feature = "unstable")]
    PRIVS_NEEDED,
    #[cfg(feature = "unstable")]
    BLANK_MSG,
    #[cfg(feature = "unstable")]
    INVALID_NICK,
    #[cfg(feature = "unstable")]
    BANNED,
    #[cfg(feature = "unstable")]
    MAX_REGISTRATIONS,
);

#[cfg(test)]
mod tests {
    #[test]
    fn values_match_wire_codes() {
        assert_eq!(super::ACCOUNT_EXISTS, "ACCOUNT_EXISTS");
        assert_eq!(super::INVALID_UTF8, "INVALID_UTF8");
        #[cfg(feature = "unstable")]
        assert_eq!(super::MAX_REGISTRATIONS, "MAX_REGISTRATIONS");
    }
}
