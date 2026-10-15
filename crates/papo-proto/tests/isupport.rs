//! Raw ISUPPORT token accumulation behavior.

use papo_proto::{Advertised, Casemapping, Isupport, MessageRef, Numeric};

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
