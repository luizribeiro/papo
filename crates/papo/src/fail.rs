//! Standard-reply `FAIL` code constants.
//!
//! Some codes have multiple wire spellings. Ergo uses `ACCOUNT_NAME_MUST_BE_NICK`
//! for account registration, `UNKNOWN_MSGID` for redaction, and `INVALID_PARAMS`
//! for history and read-marker parameters. `UnrealIRCd` also uses `INVALID_PARAMS`
//! for `CHATHISTORY`.
//!
//! papo also recognizes `ACCOUNT_NAME_MUST_MATCH_NICK`, `UNKNOWN_MESSAGE_ID`,
//! and `INVALID_PARAMETERS` for those respective refusals. Match
//! [`crate::Refusal`] to handle their shared meanings across spellings.

macro_rules! typed_codes {
    ($($name:ident),+ $(,)?) => {
        $(
            #[doc = concat!("The `", stringify!($name), "` failure code.")]
            pub const $name: crate::ReplyCode = crate::ReplyCode::constant(stringify!($name));
        )+
    };
}

typed_codes!(
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
);

typed_codes!(
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
);

#[cfg(feature = "unstable")]
typed_codes!(
    INVALID_NETID,
    INVALID_ATTRIBUTE,
    READ_ONLY_ATTRIBUTE,
    UNKNOWN_ATTRIBUTE,
    NEED_ATTRIBUTE,
    NOT_ENABLED,
    PRIVS_NEEDED,
    BLANK_MSG,
    INVALID_NICK,
    BANNED,
    MAX_REGISTRATIONS,
);

#[cfg(all(test, not(browser)))]
mod tests {
    #[test]
    fn values_match_wire_codes() {
        let account_exists: crate::ReplyCode = super::ACCOUNT_EXISTS;
        assert_eq!(account_exists, "ACCOUNT_EXISTS");
        let invalid_utf8: crate::ReplyCode = super::INVALID_UTF8;
        assert_eq!(invalid_utf8, "INVALID_UTF8");
        #[cfg(feature = "unstable")]
        {
            let max_registrations: crate::ReplyCode = super::MAX_REGISTRATIONS;
            assert_eq!(max_registrations, "MAX_REGISTRATIONS");
        }
    }
}
