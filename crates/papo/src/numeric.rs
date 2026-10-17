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
}
