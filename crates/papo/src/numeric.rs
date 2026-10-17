//! Common IRC numeric replies and errors.

macro_rules! numerics {
    ($($name:ident = $value:literal),+ $(,)?) => {
        $(
            #[doc = concat!("The `", stringify!($name), "` numeric.")]
            pub const $name: u16 = $value;
        )+
    };
}

numerics! {
    RPL_WELCOME = 1,
    RPL_YOURHOST = 2,
    RPL_CREATED = 3,
    RPL_MYINFO = 4,
    RPL_ISUPPORT = 5,
    RPL_LUSERCLIENT = 251,
    RPL_LUSEROP = 252,
    RPL_LUSERUNKNOWN = 253,
    RPL_LUSERCHANNELS = 254,
    RPL_LUSERME = 255,
    RPL_LOCALUSERS = 265,
    RPL_GLOBALUSERS = 266,
    RPL_MOTD = 372,
    RPL_MOTDSTART = 375,
    RPL_ENDOFMOTD = 376,
    ERR_NOMOTD = 422,
}

numerics! {
    RPL_WHOISCERTFP = 276,
    RPL_AWAY = 301,
    RPL_WHOISREGNICK = 307,
    RPL_WHOISUSER = 311,
    RPL_WHOISSERVER = 312,
    RPL_WHOISOPERATOR = 313,
    RPL_WHOWASUSER = 314,
    RPL_WHOISIDLE = 317,
    RPL_ENDOFWHOIS = 318,
    RPL_WHOISCHANNELS = 319,
    RPL_WHOISSPECIAL = 320,
    RPL_LISTSTART = 321,
    RPL_LIST = 322,
    RPL_LISTEND = 323,
    RPL_WHOISACCOUNT = 330,
    RPL_NOTOPIC = 331,
    RPL_TOPIC = 332,
    RPL_TOPICWHOTIME = 333,
    RPL_WHOISBOT = 335,
    RPL_WHOISACTUALLY = 338,
    RPL_NAMREPLY = 353,
    RPL_ENDOFNAMES = 366,
    RPL_ENDOFWHOWAS = 369,
    RPL_WHOISHOST = 378,
    RPL_WHOISMODES = 379,
    RPL_WHOISSECURE = 671,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_lusers_and_motd_values_match_the_wire() {
        assert_eq!(RPL_WELCOME, 1);
        assert_eq!(RPL_YOURHOST, 2);
        assert_eq!(RPL_CREATED, 3);
        assert_eq!(RPL_MYINFO, 4);
        assert_eq!(RPL_ISUPPORT, 5);
        assert_eq!(RPL_LUSERCLIENT, 251);
        assert_eq!(RPL_LUSEROP, 252);
        assert_eq!(RPL_LUSERUNKNOWN, 253);
        assert_eq!(RPL_LUSERCHANNELS, 254);
        assert_eq!(RPL_LUSERME, 255);
        assert_eq!(RPL_LOCALUSERS, 265);
        assert_eq!(RPL_GLOBALUSERS, 266);
        assert_eq!(RPL_MOTD, 372);
        assert_eq!(RPL_MOTDSTART, 375);
        assert_eq!(RPL_ENDOFMOTD, 376);
        assert_eq!(ERR_NOMOTD, 422);
    }

    #[test]
    fn query_values_match_the_wire() {
        assert_eq!(RPL_WHOISCERTFP, 276);
        assert_eq!(RPL_AWAY, 301);
        assert_eq!(RPL_WHOISREGNICK, 307);
        assert_eq!(RPL_WHOISUSER, 311);
        assert_eq!(RPL_WHOISSERVER, 312);
        assert_eq!(RPL_WHOISOPERATOR, 313);
        assert_eq!(RPL_WHOWASUSER, 314);
        assert_eq!(RPL_WHOISIDLE, 317);
        assert_eq!(RPL_ENDOFWHOIS, 318);
        assert_eq!(RPL_WHOISCHANNELS, 319);
        assert_eq!(RPL_WHOISSPECIAL, 320);
        assert_eq!(RPL_LISTSTART, 321);
        assert_eq!(RPL_LIST, 322);
        assert_eq!(RPL_LISTEND, 323);
        assert_eq!(RPL_WHOISACCOUNT, 330);
        assert_eq!(RPL_NOTOPIC, 331);
        assert_eq!(RPL_TOPIC, 332);
        assert_eq!(RPL_TOPICWHOTIME, 333);
        assert_eq!(RPL_WHOISBOT, 335);
        assert_eq!(RPL_WHOISACTUALLY, 338);
        assert_eq!(RPL_NAMREPLY, 353);
        assert_eq!(RPL_ENDOFNAMES, 366);
        assert_eq!(RPL_ENDOFWHOWAS, 369);
        assert_eq!(RPL_WHOISHOST, 378);
        assert_eq!(RPL_WHOISMODES, 379);
        assert_eq!(RPL_WHOISSECURE, 671);
    }
}
