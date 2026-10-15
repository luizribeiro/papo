use std::fmt;

use crate::Numeric;

macro_rules! commands {
    ($(#[$doc:meta] $variant:ident => $name:literal),+ $(,)?) => {
        /// An IRC command name.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[non_exhaustive]
        pub enum Command<'a> {
            $(#[$doc] $variant,)+
            /// A three-digit numeric reply or error.
            Numeric(Numeric),
            /// A command not known to this version of papo.
            Other(&'a str),
        }

        impl<'a> Command<'a> {
            /// Parses a command using ASCII case-insensitive name matching.
            #[must_use]
            pub fn parse(command: &'a str) -> Self {
                $(if command.eq_ignore_ascii_case($name) {
                    return Self::$variant;
                })+
                if command.len() == 3 && command.bytes().all(|byte| byte.is_ascii_digit()) {
                    let code = command.parse().expect("three ASCII digits fit in u16");
                    return Self::Numeric(Numeric::new(code).expect("three ASCII digits are valid"));
                }
                Self::Other(command)
            }
        }

        impl fmt::Display for Command<'_> {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $(Self::$variant => formatter.write_str($name),)+
                    Self::Numeric(numeric) => numeric.fmt(formatter),
                    Self::Other(command) => command
                        .chars()
                        .try_for_each(|character| write!(formatter, "{}", character.to_ascii_uppercase())),
                }
            }
        }

        #[cfg(test)]
        const NAMED_COMMANDS: &[(Command<'static>, &str)] = &[
            $((Command::$variant, $name),)+
        ];
    };
}

commands! {
    /// `ADMIN`.
    Admin => "ADMIN",
    /// `AWAY`.
    Away => "AWAY",
    /// `CONNECT`.
    Connect => "CONNECT",
    /// `ERROR`.
    Error => "ERROR",
    /// `INFO`.
    Info => "INFO",
    /// `INVITE`.
    Invite => "INVITE",
    /// `ISON`.
    Ison => "ISON",
    /// `JOIN`.
    Join => "JOIN",
    /// `KICK`.
    Kick => "KICK",
    /// `KILL`.
    Kill => "KILL",
    /// `LINKS`.
    Links => "LINKS",
    /// `LIST`.
    List => "LIST",
    /// `LUSERS`.
    Lusers => "LUSERS",
    /// `MODE`.
    Mode => "MODE",
    /// `MOTD`.
    Motd => "MOTD",
    /// `NAMES`.
    Names => "NAMES",
    /// `NICK`.
    Nick => "NICK",
    /// `NOTICE`.
    Notice => "NOTICE",
    /// `OPER`.
    Oper => "OPER",
    /// `PART`.
    Part => "PART",
    /// `PASS`.
    Pass => "PASS",
    /// `PING`.
    Ping => "PING",
    /// `PONG`.
    Pong => "PONG",
    /// `PRIVMSG`.
    Privmsg => "PRIVMSG",
    /// `QUIT`.
    Quit => "QUIT",
    /// `REHASH`.
    Rehash => "REHASH",
    /// `RESTART`.
    Restart => "RESTART",
    /// `SQUIT`.
    Squit => "SQUIT",
    /// `STATS`.
    Stats => "STATS",
    /// `TIME`.
    Time => "TIME",
    /// `TOPIC`.
    Topic => "TOPIC",
    /// `TRACE`.
    Trace => "TRACE",
    /// `USER`.
    User => "USER",
    /// `USERHOST`.
    Userhost => "USERHOST",
    /// `VERSION`.
    Version => "VERSION",
    /// `WALLOPS`.
    Wallops => "WALLOPS",
    /// `WHO`.
    Who => "WHO",
    /// `WHOIS`.
    Whois => "WHOIS",
    /// `WHOWAS`.
    Whowas => "WHOWAS",
    /// `CAP`.
    Cap => "CAP",
    /// `AUTHENTICATE`.
    Authenticate => "AUTHENTICATE",
    /// `BATCH`.
    Batch => "BATCH",
    /// `TAGMSG`.
    Tagmsg => "TAGMSG",
    /// `SETNAME`.
    Setname => "SETNAME",
    /// `CHGHOST`.
    Chghost => "CHGHOST",
    /// `ACCOUNT`.
    Account => "ACCOUNT",
    /// `MONITOR`.
    Monitor => "MONITOR",
    /// `CHATHISTORY`.
    Chathistory => "CHATHISTORY",
    /// `MARKREAD`.
    Markread => "MARKREAD",
    /// `REDACT`.
    Redact => "REDACT",
    /// `RENAME`.
    Rename => "RENAME",
    /// `METADATA`.
    Metadata => "METADATA",
    /// `REGISTER`.
    Register => "REGISTER",
    /// `VERIFY`.
    Verify => "VERIFY",
    /// `ISUPPORT`.
    Isupport => "ISUPPORT",
    /// `PERSISTENCE`.
    Persistence => "PERSISTENCE",
    /// `WEBPUSH`.
    Webpush => "WEBPUSH",
    /// `FAIL`.
    Fail => "FAIL",
    /// `WARN`.
    Warn => "WARN",
    /// `NOTE`.
    Note => "NOTE",
    /// `RELAYMSG`.
    Relaymsg => "RELAYMSG",
    /// `EXTJWT`.
    Extjwt => "EXTJWT",
}

#[cfg(test)]
mod tests {
    use super::{Command, NAMED_COMMANDS};
    use crate::Numeric;

    #[test]
    fn every_named_command_round_trips_case_insensitively() {
        for &(command, name) in NAMED_COMMANDS {
            assert_eq!(Command::parse(name), command);
            assert_eq!(Command::parse(&name.to_ascii_lowercase()), command);
            assert_eq!(command.to_string(), name);
        }
    }

    #[test]
    fn parses_numerics_and_preserves_unknown_commands() {
        assert_eq!(
            Command::parse("005"),
            Command::Numeric(Numeric::new(5).unwrap())
        );
        assert_eq!(Command::parse("42"), Command::Other("42"));
        assert_eq!(Command::parse("1000"), Command::Other("1000"));
        assert_eq!(Command::parse("future").to_string(), "FUTURE");
    }
}
