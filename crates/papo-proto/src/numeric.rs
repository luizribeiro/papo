use std::fmt;

/// A three-digit IRC numeric reply or error.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Numeric(u16);

impl Numeric {
    /// Creates a numeric from a value in the three-digit range.
    #[must_use]
    pub const fn new(code: u16) -> Option<Self> {
        if code <= 999 { Some(Self(code)) } else { None }
    }

    /// Returns the numeric code without leading zeroes.
    #[must_use]
    pub const fn code(self) -> u16 {
        self.0
    }
}

macro_rules! numerics {
    ($($name:ident = $value:literal),+ $(,)?) => {
        impl Numeric {
            $(
                #[doc = concat!("The `", stringify!($name), "` (`", stringify!($value), "`) numeric.")]
                pub const $name: Self = Self($value);
            )+

            /// Returns the conventional name of this numeric, when known.
            #[must_use]
            pub const fn name(self) -> Option<&'static str> {
                match self {
                    $(Self::$name => Some(stringify!($name)),)+
                    _ => None,
                }
            }
        }

        #[cfg(test)]
        const NAMED_NUMERICS: &[(Numeric, &str)] = &[
            $((Numeric::$name, stringify!($name)),)+
        ];
    };
}

numerics! {
    RPL_WELCOME = 1,
    RPL_YOURHOST = 2,
    RPL_CREATED = 3,
    RPL_MYINFO = 4,
    RPL_ISUPPORT = 5,
    RPL_BOUNCE = 10,
    RPL_REMOTEISUPPORT = 105,
    RPL_STATSCOMMANDS = 212,
    RPL_ENDOFSTATS = 219,
    RPL_UMODEIS = 221,
    RPL_STATSUPTIME = 242,
    RPL_LUSERCLIENT = 251,
    RPL_LUSEROP = 252,
    RPL_LUSERUNKNOWN = 253,
    RPL_LUSERCHANNELS = 254,
    RPL_LUSERME = 255,
    RPL_ADMINME = 256,
    RPL_ADMINLOC1 = 257,
    RPL_ADMINLOC2 = 258,
    RPL_ADMINEMAIL = 259,
    RPL_TRYAGAIN = 263,
    RPL_LOCALUSERS = 265,
    RPL_GLOBALUSERS = 266,
    RPL_WHOISCERTFP = 276,
    RPL_AWAY = 301,
    RPL_USERHOST = 302,
    RPL_ISON = 303,
    RPL_UNAWAY = 305,
    RPL_NOWAWAY = 306,
    RPL_WHOISREGNICK = 307,
    RPL_WHOISUSER = 311,
    RPL_WHOISSERVER = 312,
    RPL_WHOISOPERATOR = 313,
    RPL_WHOWASUSER = 314,
    RPL_ENDOFWHO = 315,
    RPL_WHOISIDLE = 317,
    RPL_ENDOFWHOIS = 318,
    RPL_WHOISCHANNELS = 319,
    RPL_WHOISSPECIAL = 320,
    RPL_LISTSTART = 321,
    RPL_LIST = 322,
    RPL_LISTEND = 323,
    RPL_CHANNELMODEIS = 324,
    RPL_CREATIONTIME = 329,
    RPL_WHOISACCOUNT = 330,
    RPL_NOTOPIC = 331,
    RPL_TOPIC = 332,
    RPL_TOPICWHOTIME = 333,
    RPL_WHOISBOT = 335,
    RPL_INVITELIST = 336,
    RPL_ENDOFINVITELIST = 337,
    RPL_WHOISACTUALLY = 338,
    RPL_INVITING = 341,
    RPL_INVEXLIST = 346,
    RPL_ENDOFINVEXLIST = 347,
    RPL_EXCEPTLIST = 348,
    RPL_ENDOFEXCEPTLIST = 349,
    RPL_VERSION = 351,
    RPL_WHOREPLY = 352,
    RPL_NAMREPLY = 353,
    RPL_WHOSPCRPL = 354,
    RPL_LINKS = 364,
    RPL_ENDOFLINKS = 365,
    RPL_ENDOFNAMES = 366,
    RPL_BANLIST = 367,
    RPL_ENDOFBANLIST = 368,
    RPL_ENDOFWHOWAS = 369,
    RPL_INFO = 371,
    RPL_MOTD = 372,
    RPL_ENDOFINFO = 374,
    RPL_MOTDSTART = 375,
    RPL_ENDOFMOTD = 376,
    RPL_WHOISHOST = 378,
    RPL_WHOISMODES = 379,
    RPL_YOUREOPER = 381,
    RPL_REHASHING = 382,
    RPL_TIME = 391,
    RPL_VISIBLEHOST = 396,
    ERR_UNKNOWNERROR = 400,
    ERR_NOSUCHNICK = 401,
    ERR_NOSUCHSERVER = 402,
    ERR_NOSUCHCHANNEL = 403,
    ERR_CANNOTSENDTOCHAN = 404,
    ERR_TOOMANYCHANNELS = 405,
    ERR_WASNOSUCHNICK = 406,
    ERR_NOORIGIN = 409,
    ERR_NORECIPIENT = 411,
    ERR_NOTEXTTOSEND = 412,
    ERR_INPUTTOOLONG = 417,
    ERR_UNKNOWNCOMMAND = 421,
    ERR_NOMOTD = 422,
    ERR_NONICKNAMEGIVEN = 431,
    ERR_ERRONEUSNICKNAME = 432,
    ERR_NICKNAMEINUSE = 433,
    ERR_NICKCOLLISION = 436,
    ERR_UNAVAILRESOURCE = 437,
    ERR_USERNOTINCHANNEL = 441,
    ERR_NOTONCHANNEL = 442,
    ERR_USERONCHANNEL = 443,
    ERR_NOTREGISTERED = 451,
    ERR_NEEDMOREPARAMS = 461,
    ERR_ALREADYREGISTERED = 462,
    ERR_PASSWDMISMATCH = 464,
    ERR_YOUREBANNEDCREEP = 465,
    ERR_LINKCHANNEL = 470,
    ERR_CHANNELISFULL = 471,
    ERR_UNKNOWNMODE = 472,
    ERR_INVITEONLYCHAN = 473,
    ERR_BANNEDFROMCHAN = 474,
    ERR_BADCHANNELKEY = 475,
    ERR_BADCHANMASK = 476,
    ERR_NOPRIVILEGES = 481,
    ERR_CHANOPRIVSNEEDED = 482,
    ERR_CANTKILLSERVER = 483,
    ERR_SECUREONLYCHAN = 489,
    ERR_NOOPERHOST = 491,
    ERR_UMODEUNKNOWNFLAG = 501,
    ERR_USERSDONTMATCH = 502,
    ERR_INVALIDKEY = 525,
    RPL_WHOISSECURE = 671,
    ERR_INVALIDMODEPARAM = 696,
    ERR_NOPRIVS = 723,
    RPL_MONONLINE = 730,
    RPL_MONOFFLINE = 731,
    RPL_MONLIST = 732,
    RPL_ENDOFMONLIST = 733,
    ERR_MONLISTFULL = 734,
    RPL_WHOISKEYVALUE = 760,
    RPL_KEYVALUE = 761,
    RPL_KEYNOTSET = 766,
    RPL_METADATASUBOK = 770,
    RPL_METADATAUNSUBOK = 771,
    RPL_METADATASUBS = 772,
    RPL_METADATASYNCLATER = 774,
    RPL_LOGGEDIN = 900,
    RPL_LOGGEDOUT = 901,
    ERR_NICKLOCKED = 902,
    RPL_SASLSUCCESS = 903,
    ERR_SASLFAIL = 904,
    ERR_SASLTOOLONG = 905,
    ERR_SASLABORTED = 906,
    ERR_SASLALREADY = 907,
    RPL_SASLMECHS = 908,
}

impl Numeric {
    /// Reports whether this numeric represents an error.
    #[must_use]
    pub const fn is_error(self) -> bool {
        (self.0 >= 400 && self.0 <= 599)
            || matches!(self.0, 691 | 697 | 698)
            || matches!(
                self,
                Self::ERR_INVALIDMODEPARAM
                    | Self::ERR_NOPRIVS
                    | Self::ERR_MONLISTFULL
                    | Self::ERR_NICKLOCKED
                    | Self::ERR_SASLFAIL
                    | Self::ERR_SASLTOOLONG
                    | Self::ERR_SASLABORTED
                    | Self::ERR_SASLALREADY
            )
    }
}

impl fmt::Display for Numeric {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:03}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use crate::Command;

    use super::{NAMED_NUMERICS, Numeric};

    fn numeric(code: u16) -> Numeric {
        Numeric::new(code).unwrap()
    }

    #[test]
    fn every_named_numeric_round_trips() {
        for &(numeric, name) in NAMED_NUMERICS {
            assert_eq!(numeric.name(), Some(name));
            assert_eq!(
                Command::parse(&numeric.to_string()),
                Command::Numeric(numeric)
            );
        }
        assert_eq!(numeric(999).name(), None);
    }

    #[test]
    fn validates_the_three_digit_range() {
        assert_eq!(Numeric::new(0).map(Numeric::code), Some(0));
        assert_eq!(Numeric::new(999).map(Numeric::code), Some(999));
        assert_eq!(Numeric::new(1000), None);
        assert_eq!(Numeric::new(u16::MAX), None);
    }

    #[test]
    fn classifies_error_range_boundaries() {
        assert!(!numeric(399).is_error());
        assert!(numeric(400).is_error());
        assert!(numeric(599).is_error());
        assert!(!numeric(600).is_error());
    }

    #[test]
    fn classifies_documented_errors_outside_the_error_ranges() {
        let errors = [691, 696, 697, 698, 723, 734, 902, 904, 905, 906, 907];
        for value in errors {
            assert!(numeric(value).is_error(), "{value}");
        }

        let replies = [695, 699, 722, 724, 733, 735, 901, 903, 908];
        for value in replies {
            assert!(!numeric(value).is_error(), "{value}");
        }
    }
}
