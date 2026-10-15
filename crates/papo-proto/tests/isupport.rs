//! Raw ISUPPORT token accumulation behavior.

use papo_proto::{
    Advertised, Casemapping, ChanModes, Isupport, Limit, LimitValue, MessageRef, Numeric, Prefix,
};

#[test]
fn accumulates_local_tokens_and_ignores_remote_isupport() {
    let local = MessageRef::parse(
        ":ergo.test 005 tester CASEMAPPING=ascii WHOX NETWORK=ErgoTest :are supported by this server",
    )
    .unwrap();
    let remote_line = format!(
        ":irc.remote.example {} tester CASEMAPPING=rfc1459 NETWORK=Remote :are supported by this server",
        Numeric::RPL_REMOTEISUPPORT
    );
    let remote = MessageRef::parse(&remote_line).unwrap();
    let mut support = Isupport::default();
    support.apply(&local);
    support.apply(&remote);

    assert_eq!(support.get("CASEMAPPING"), Advertised::Value("ascii"));
    assert_eq!(support.get("whox"), Advertised::Bare);
    assert_eq!(support.get("NETWORK"), Advertised::Value("ErgoTest"));
    assert_eq!(support.get("tester"), Advertised::Absent);
    assert_eq!(
        support.get("are supported by this server"),
        Advertised::Absent
    );
}

#[test]
fn negation_removes_tokens_and_readvertisement_overwrites() {
    let mut support = Isupport::default();
    support.apply_tokens(["NETWORK=old", "WHOX", "-MISSING"]);
    support.apply_tokens(["NETWORK=new", "-WHOX"]);

    assert_eq!(support.get("NETWORK"), Advertised::Value("new"));
    assert_eq!(support.get("WHOX"), Advertised::Absent);
    assert_eq!(support.get("MISSING"), Advertised::Absent);
}

#[test]
fn unescapes_values_in_one_pass() {
    let mut support = Isupport::default();
    support.apply_tokens([
        "NETWORK=Example\\x20Net\\x3Dprod",
        "BACKSLASH_EQUALS=\\x5Cx3D",
        "BACKSLASH_SPACE=\\x5Cx20",
        "TRAILING=tail\\x5",
        "EMPTY=",
    ]);

    assert_eq!(
        support.get("NETWORK"),
        Advertised::Value("Example Net=prod")
    );
    assert_eq!(support.get("BACKSLASH_EQUALS"), Advertised::Value("\\x3D"));
    assert_eq!(support.get("BACKSLASH_SPACE"), Advertised::Value("\\x20"));
    assert_eq!(support.get("TRAILING"), Advertised::Value("tail\\x5"));
    assert_eq!(support.get("EMPTY"), Advertised::Value(""));
}

#[test]
fn empty_and_malformed_tokens_do_not_create_keys() {
    let mut support = Isupport::default();
    support.apply_tokens(std::iter::empty::<&str>());
    support.apply_tokens(["", "=missing-key", "-"]);

    assert_eq!(support.get(""), Advertised::Absent);
}

#[test]
fn selects_the_advertised_casemapping_with_the_protocol_default() {
    let mut support = Isupport::default();
    assert_eq!(support.casemapping(), Casemapping::Rfc1459);

    support.apply_tokens(["CASEMAPPING=ascii"]);
    assert_eq!(support.casemapping(), Casemapping::Ascii);
    support.apply_tokens(["CASEMAPPING=rfc1459-strict"]);
    assert_eq!(support.casemapping(), Casemapping::Rfc1459Strict);
    support.apply_tokens(["CASEMAPPING=unknown"]);
    assert_eq!(support.casemapping(), Casemapping::Rfc1459);
    support.apply_tokens(["-CASEMAPPING"]);
    assert_eq!(support.casemapping(), Casemapping::Rfc1459);
}

#[cfg(feature = "precis")]
#[test]
fn ergo_utf8mapping_selects_precis_when_compiled_in() {
    let mut support = Isupport::default();
    support.apply_tokens(["CASEMAPPING=ascii", "UTF8MAPPING=rfc8265"]);
    assert_eq!(support.casemapping(), Casemapping::Precis);
}

#[cfg(not(feature = "precis"))]
#[test]
fn ergo_utf8mapping_keeps_ascii_without_precis() {
    let mut support = Isupport::default();
    support.apply_tokens(["CASEMAPPING=ascii", "UTF8MAPPING=rfc8265"]);
    assert_eq!(support.casemapping(), Casemapping::Ascii);
}

#[test]
fn parses_ergo_channel_shape_and_ranked_prefixes() {
    let mut support = Isupport::default();
    support.apply_tokens([
        "CHANTYPES=#",
        "PREFIX=(qaohv)~&@%+",
        "CHANMODES=Ibe,k,fl,CEMRUimnstu,ignored",
        "STATUSMSG=~&@%+",
    ]);

    assert_eq!(support.chantypes(), "#");
    assert!(support.is_channel("#ergo"));
    assert!(!support.is_channel("&local"));
    assert_eq!(
        support.prefix(),
        vec![
            prefix('q', '~', 0),
            prefix('a', '&', 1),
            prefix('o', '@', 2),
            prefix('h', '%', 3),
            prefix('v', '+', 4),
        ]
    );
    assert_eq!(
        support.chanmodes(),
        Some(ChanModes {
            list: "Ibe",
            always: "k",
            when_set: "fl",
            never: "CEMRUimnstu",
        })
    );
    assert_eq!(support.statusmsg(), Some("~&@%+"));
}

#[test]
fn channel_shape_honors_defaults_empty_values_and_malformed_prefixes() {
    let mut support = Isupport::default();
    assert_eq!(support.chantypes(), "#&");
    assert!(support.is_channel("&local"));
    assert_eq!(
        support.prefix(),
        vec![prefix('o', '@', 0), prefix('v', '+', 1)]
    );

    support.apply_tokens(["CHANTYPES=", "PREFIX="]);
    assert_eq!(support.chantypes(), "");
    assert!(!support.is_channel("#empty"));
    assert!(support.prefix().is_empty());

    support.apply_tokens(["PREFIX=(ov)@", "CHANMODES=a,b,c"]);
    assert_eq!(
        support.prefix(),
        vec![prefix('o', '@', 0), prefix('v', '+', 1)]
    );
    assert_eq!(support.chanmodes(), None);
}

#[test]
fn parses_ergo_limits_and_lengths() {
    let mut support = Isupport::default();
    support.apply_tokens([
        "AWAYLEN=390",
        "CHANLIMIT=#:100",
        "CHANNELLEN=64",
        "KICKLEN=390",
        "MAXLIST=beI:100",
        "MAXTARGETS=4",
        "MODES",
        "NAMELEN=150",
        "NICKLEN=32",
        "TARGMAX=NAMES:1,LIST:1,KICK:,WHOIS:1,USERHOST:10,PRIVMSG:4,TAGMSG:4,NOTICE:4,MONITOR:100",
        "TOPICLEN=390",
    ]);

    assert_eq!(support.awaylen(), Some(390));
    assert_eq!(support.chanlimit(), vec![limit("#", Some(100))]);
    assert_eq!(support.channellen(), Some(64));
    assert_eq!(support.kicklen(), Some(390));
    assert_eq!(support.maxlist(), vec![limit("beI", Some(100))]);
    assert_eq!(support.maxtargets(), Some(LimitValue::Limited(4)));
    assert_eq!(support.modes(), None);
    assert_eq!(support.namelen(), Some(150));
    assert_eq!(support.nicklen(), Some(32));
    assert_eq!(
        support.targmax(),
        vec![
            limit("NAMES", Some(1)),
            limit("LIST", Some(1)),
            limit("KICK", None),
            limit("WHOIS", Some(1)),
            limit("USERHOST", Some(10)),
            limit("PRIVMSG", Some(4)),
            limit("TAGMSG", Some(4)),
            limit("NOTICE", Some(4)),
            limit("MONITOR", Some(100)),
        ]
    );
    assert!(support.targmax()[5].name.eq_ignore_ascii_case("privmsg"));
    assert_eq!(support.topiclen(), Some(390));
}

#[test]
fn limits_apply_defaults_and_reject_malformed_values() {
    let mut support = Isupport::default();
    assert_eq!(support.modes(), Some(3));
    assert_eq!(
        support.targmax(),
        vec![limit("JOIN", None), limit("PART", None)]
    );

    support.apply_tokens([
        "TARGMAX=PRIVMSG:not-a-number",
        "MAXTARGETS=0",
        "MODES=invalid",
        "CHANLIMIT=#:0",
        "MAXLIST=beI:",
        "NICKLEN=0",
    ]);
    assert_eq!(
        support.targmax(),
        vec![limit("JOIN", None), limit("PART", None)]
    );
    assert_eq!(support.maxtargets(), None);
    assert_eq!(support.modes(), Some(3));
    assert!(support.chanlimit().is_empty());
    assert!(support.maxlist().is_empty());
    assert_eq!(support.nicklen(), None);
}

fn prefix(mode: char, symbol: char, rank: usize) -> Prefix {
    Prefix { mode, symbol, rank }
}

fn limit(name: &str, limit: Option<usize>) -> Limit<'_> {
    Limit { name, limit }
}
