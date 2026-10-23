//! Raw ISUPPORT token accumulation behavior.

use papo_proto::{
    Advertised, Casemapping, ChanModes, Extban, Isupport, Limit, LimitValue, LineRef, Numeric,
    Prefix,
};

#[test]
fn accumulates_local_tokens_and_ignores_remote_isupport() {
    let local = LineRef::parse(
        ":ergo.test 005 tester CASEMAPPING=ascii WHOX NETWORK=ErgoTest :are supported by this server",
    )
    .unwrap();
    let remote_line = format!(
        ":irc.remote.example {} tester CASEMAPPING=rfc1459 NETWORK=Remote :are supported by this server",
        Numeric::RPL_REMOTEISUPPORT
    );
    let remote = LineRef::parse(&remote_line).unwrap();
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
fn iterates_over_the_complete_accumulated_token_set() {
    let mut support = Isupport::default();
    support.apply_tokens([
        "CASEMAPPING=ascii",
        "UTF8ONLY",
        "NETWORK=Ergo\\x20Test",
        "-UTF8ONLY",
    ]);

    let mut tokens = support.tokens().collect::<Vec<_>>();
    tokens.sort_unstable_by_key(|(key, _)| *key);
    assert_eq!(
        tokens,
        [
            ("CASEMAPPING", Advertised::Value("ascii")),
            ("NETWORK", Advertised::Value("Ergo Test")),
        ]
    );
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
fn expands_the_network_icon_size_template() {
    let mut support = Isupport::default();
    assert_eq!(support.icon(), None);
    assert_eq!(support.icon_url(64), None);

    support.apply_tokens(["draft/ICON=https://cdn.example.net/icon-{size}.png"]);
    assert_eq!(
        support.icon(),
        Some("https://cdn.example.net/icon-{size}.png")
    );
    assert_eq!(
        support.icon_url(64).as_deref(),
        Some("https://cdn.example.net/icon-64.png")
    );

    support.apply_tokens(["draft/ICON=https://cdn.example.net/network.png"]);
    assert_eq!(
        support.icon_url(128).as_deref(),
        Some("https://cdn.example.net/network.png")
    );
}

#[test]
fn reads_the_webpush_vapid_key() {
    let mut support = Isupport::default();
    assert_eq!(support.vapid(), None);
    support.apply_tokens(["VAPID=BN2Zr6VZlaXr1w"]);
    assert_eq!(support.vapid(), Some("BN2Zr6VZlaXr1w"));
}

#[test]
fn empty_and_malformed_tokens_do_not_create_keys() {
    let mut support = Isupport::default();
    support.apply_tokens(std::iter::empty::<&str>());
    support.apply_tokens(["", "=missing-key", "-"]);

    assert_eq!(support.get(""), Advertised::Absent);
}

#[test]
fn bounds_tokens_and_values_while_retaining_known_features() {
    let mut support = Isupport::default();
    support.apply_tokens((0..5_000).map(|index| format!("VENDOR{index}=present")));
    support.apply_tokens([
        format!("NETWORK={}", "n".repeat(5_000)),
        "CASEMAPPING=ascii".to_owned(),
        "PREFIX=(qaohv)~&@%+".to_owned(),
        "CHANTYPES=#".to_owned(),
    ]);

    assert_eq!(support.tokens().len(), 1_024);
    assert_eq!(support.network().unwrap().len(), 4_096);
    assert_eq!(support.casemapping(), Casemapping::Ascii);
    assert_eq!(support.prefix().len(), 5);
    assert_eq!(support.chantypes(), "#");
    assert!(support.__limit_reached());
}

#[test]
fn token_limit_accepts_the_literal_boundary_and_rejects_one_over() {
    for (count, retained, limited) in [
        (1_023, 1_023, false),
        (1_024, 1_024, false),
        (1_025, 1_024, true),
    ] {
        let mut support = Isupport::default();
        support
            .apply_tokens((0..count).map(|index| format!("VENDOR.EXAMPLE/TOKEN{index}=present")));
        assert_eq!(support.tokens().len(), retained, "input count {count}");
        assert_eq!(support.__limit_reached(), limited, "input count {count}");
    }
}

#[test]
fn isupport_values_keep_literal_byte_boundaries() {
    for (value, expected, limited) in [
        ("x".repeat(4_095), "x".repeat(4_095), false),
        ("x".repeat(4_096), "x".repeat(4_096), false),
        ("x".repeat(4_097), "x".repeat(4_096), true),
        (format!("{}é", "x".repeat(4_095)), "x".repeat(4_095), true),
    ] {
        let mut support = Isupport::default();
        support.apply_tokens([format!("VENDOR.EXAMPLE/STATE={value}")]);
        assert_eq!(
            support.get("VENDOR.EXAMPLE/STATE"),
            Advertised::Value(&expected)
        );
        assert_eq!(support.__limit_reached(), limited);
    }
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
fn distinguishes_unlimited_absent_and_numeric_target_limits() {
    let mut support = Isupport::default();
    support.apply(&LineRef::parse(
        ":ergo.test 005 papo TARGMAX=NAMES:1,LIST:1,KICK:,WHOIS:1,USERHOST:10,PRIVMSG:4,TAGMSG:4,NOTICE:4,MONITOR:100 :are supported by this server",
    ).unwrap());

    let limits = support.targmax();
    assert_eq!(
        limits
            .iter()
            .find(|limit| limit.name == "KICK")
            .unwrap()
            .limit,
        Some(LimitValue::Unlimited)
    );
    assert!(limits.iter().all(|limit| limit.name != "AWAY"));
    assert_eq!(
        limits
            .iter()
            .find(|limit| limit.name == "PRIVMSG")
            .unwrap()
            .limit,
        Some(LimitValue::Limited(4))
    );
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

#[test]
fn parses_ergo_list_ban_history_and_flag_tokens() {
    let mut support = Isupport::default();
    support.apply_tokens([
        "BOT=B",
        "CHATHISTORY=1000",
        "draft/CHATHISTORY=1000",
        "ELIST=U",
        "EXCEPTS",
        "EXTBAN=,m",
        "INVEX",
        "MSGREFTYPES=msgid,timestamp",
        "MONITOR=100",
        "NETWORK=ErgoTest",
        "SAFELIST",
        "UTF8ONLY",
        "WHOX",
    ]);

    assert_eq!(support.bot_mode(), Some('B'));
    assert_eq!(support.chathistory(), Some(LimitValue::Limited(1000)));
    assert_eq!(support.elist(), Some("U"));
    assert_eq!(support.excepts(), Some('e'));
    assert_eq!(
        support.extban(),
        Some(Extban {
            prefix: None,
            types: "m",
        })
    );
    assert_eq!(support.invex(), Some('I'));
    assert_eq!(support.msgreftypes(), vec!["msgid", "timestamp"]);
    assert_eq!(support.monitor(), Some(LimitValue::Limited(100)));
    assert_eq!(support.network(), Some("ErgoTest"));
    assert!(support.safelist());
    assert!(support.utf8only());
    assert!(support.whox());
}

#[test]
fn parses_solanum_libera_advertisements() {
    let mut support = Isupport::default();
    support.apply_tokens([
        "CASEMAPPING=rfc1459",
        "CHANLIMIT=#:250",
        "CHANMODES=eIbq,k,flj,CFLMPQScgimnprstuz,ignored",
        "CHANNELLEN=50",
        "CHANTYPES=#",
        "ELIST=CMNTU",
        "EXCEPTS=e",
        "EXTBAN=$,ajrxz",
        "INVEX=I",
        "MAXLIST=bqeI:100",
        "MODES=4",
        "NETWORK=Libera.Chat",
        "NICKLEN=16",
        "PREFIX=(ov)@+",
        "SAFELIST",
        "STATUSMSG=@+",
        "TARGMAX=NAMES:1,LIST:1,KICK:1,WHOIS:1,PRIVMSG:4,NOTICE:4,ACCEPT:,MONITOR:",
        "TOPICLEN=390",
    ]);

    assert_eq!(support.casemapping(), Casemapping::Rfc1459);
    assert_eq!(support.chanlimit(), vec![limit("#", Some(250))]);
    assert_eq!(support.chanmodes().unwrap().never, "CFLMPQScgimnprstuz");
    assert_eq!(support.extban().unwrap().prefix, Some('$'));
    assert_eq!(support.maxlist(), vec![limit("bqeI", Some(100))]);
    assert_eq!(support.modes(), Some(4));
    assert_eq!(support.network(), Some("Libera.Chat"));
    assert_eq!(support.nicklen(), Some(16));
    assert_eq!(support.statusmsg(), Some("@+"));
    assert_eq!(support.topiclen(), Some(390));
}

#[test]
fn extension_defaults_and_policies_are_typed() {
    let mut support = Isupport::default();
    assert_eq!(support.msgreftypes(), vec!["msgid", "timestamp"]);
    assert!(!support.account_required());
    assert!(!support.clienttagdeny().denies("+typing"));

    support.apply_tokens([
        "CLIENTTAGDENY=*,-typing,-example/react",
        "draft/ACCOUNTREQUIRED",
        "draft/CHATHISTORY=0",
        "ACCOUNTEXTBAN=account,a",
        "MONITOR",
    ]);
    assert!(support.account_required());
    assert_eq!(support.chathistory(), Some(LimitValue::Unlimited));
    assert_eq!(support.monitor(), Some(LimitValue::Unlimited));
    assert_eq!(support.accountextban(), vec!["account", "a"]);
    assert!(!support.clienttagdeny().denies("+typing"));
    assert!(!support.clienttagdeny().denies("example/react"));
    assert!(support.clienttagdeny().denies("+draft/reply"));

    support.apply_tokens(["CLIENTTAGDENY=typing"]);
    assert!(support.clienttagdeny().denies("+typing"));
    assert!(!support.clienttagdeny().denies("+draft/reply"));
}

fn prefix(mode: char, symbol: char, rank: usize) -> Prefix {
    Prefix { mode, symbol, rank }
}

fn limit(name: &str, limit: Option<usize>) -> Limit<'_> {
    Limit {
        name,
        limit: Some(limit.map_or(LimitValue::Unlimited, LimitValue::Limited)),
    }
}
