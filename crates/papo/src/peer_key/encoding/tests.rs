use super::{InvalidPeerKey, parse, percent_decode, percent_encode};

#[test]
fn percent_encoding_round_trips_utf8_and_delimiters() {
    let encoded = percent_encode("josé;net=id%");
    assert_eq!(encoded, "jos%C3%A9%3Bnet%3Did%25");
    assert_eq!(percent_decode(&encoded).as_deref(), Ok("josé;net=id%"));
}

#[test]
fn malformed_percent_encoding_is_rejected() {
    for value in ["%", "%0", "%GG", "%FF"] {
        assert_eq!(percent_decode(value), Err(InvalidPeerKey));
    }
}

#[test]
fn proof_kind_must_match_the_proof_fields() {
    for fields in [
        "server=irc.ergo.chat;nick-casemap=rfc1459;nick=%5BAlice%5D;proof=account",
        "server=irc.ergo.chat;nick-casemap=rfc1459;nick=%5BAlice%5D;proof=unverified;account-casemap=rfc1459;account=alice",
        "server=irc.ergo.chat;nick-casemap=rfc1459;nick=%5BAlice%5D;proof=continuity;account-casemap=rfc1459;account=alice;id=ergo-account-42",
    ] {
        assert!(matches!(
            parse(&format!("papo-peer:1;{fields}")),
            Err(InvalidPeerKey)
        ));
    }
}

#[test]
fn every_bouncer_scope_part_is_required() {
    for scope in [
        "server=;bouncer-login=alice%40example;netid=libera;upstream=irc.libera.chat",
        "server=bouncer.example;bouncer-login=;netid=libera;upstream=irc.libera.chat",
        "server=bouncer.example;bouncer-login=alice%40example;netid=;upstream=irc.libera.chat",
        "server=bouncer.example;bouncer-login=alice%40example;netid=libera;upstream=",
    ] {
        let encoded = format!(
            "papo-peer:1;{scope};nick-casemap=rfc1459;nick=%5BAlice%5D;proof=account;account-casemap=rfc1459;account=alice"
        );
        assert!(matches!(parse(&encoded), Err(InvalidPeerKey)));
    }
}

#[test]
fn network_scope_requires_its_name_and_following_fields() {
    for encoded in [
        "papo-peer:1;host=irc.ergo.chat;nick-casemap=rfc1459;nick=%5BAlice%5D;proof=unverified",
        "papo-peer:1;server=irc.ergo.chat",
    ] {
        assert!(matches!(parse(encoded), Err(InvalidPeerKey)));
    }
}
