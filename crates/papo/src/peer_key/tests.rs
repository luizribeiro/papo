use super::{InvalidPeerKey, PeerKey};

fn parse(fields: &str) -> PeerKey {
    format!("papo-peer:1;{fields}").parse().unwrap()
}

#[test]
fn peer_key_round_trips_scope_nick_and_proof() {
    let keys = [
        parse(
            "server=bouncer.example;bouncer-login=emily%40example;netid=libera;upstream=irc.libera.chat;nick-casemap=rfc1459;nick=%5Bemily%5D;proof=account;account-casemap=rfc1459;account=emily;id=account-42",
        ),
        parse("connection=Origin%2842%29;nick-casemap=ascii;nick=guest-4821;proof=unverified"),
    ];

    for key in keys {
        let encoded = key.to_string();
        let decoded = encoded.parse::<PeerKey>().unwrap();
        assert_eq!(decoded.to_string(), encoded);
        assert_eq!(decoded.is_verified(), key.is_verified());
    }
}

#[test]
fn peer_key_proof_is_not_constructible() {
    for fields in [
        "server=irc.example;nick-casemap=rfc1459;nick=alice;proof=continuity;token=7",
        "server=irc.example;nick-casemap=rfc1459;nick=alice;proof=account;account-casemap=rfc1459;account=alice;continuity=7",
        "connection=Origin%2842%29;nick-casemap=rfc1459;nick=alice;proof=account;account-casemap=rfc1459;account=alice",
    ] {
        assert!(format!("papo-peer:1;{fields}").parse::<PeerKey>().is_err());
    }
}

#[test]
fn proven_and_peer_key_proof_are_private() {
    peer_key_proof_is_not_constructible();
}

#[test]
fn persisted_peer_accounts_are_canonicalized() {
    let key = "papo-peer:1;server=irc.example;nick-casemap=rfc1459;nick=Alice%5BOps%5D;proof=account;account-casemap=rfc1459;account=Alice%5BOps%5D"
        .parse::<PeerKey>()
        .unwrap();
    assert_eq!(
        key.as_verified_str(),
        Some(
            "papo-peer:1;server=irc.example;nick-casemap=rfc1459;nick=alice%7Bops%7D;proof=account;account-casemap=rfc1459;account=alice%7Bops%7D"
        )
    );
}

#[test]
fn malformed_persisted_peer_keys_are_rejected() {
    for value in [
        "",
        "papo-peer-v1:rfc1459:4:Ergoalice",
        "papo-peer:2;server=irc.example;nick-casemap=rfc1459;nick=alice;proof=unverified",
        "papo-peer:1;server=irc.example;nick=alice;nick-casemap=rfc1459;proof=unverified",
        "papo-peer:1;server=irc.example;nick-casemap=unknown;nick=alice;proof=unverified",
        "papo-peer:1;server=;nick-casemap=rfc1459;nick=alice;proof=unverified",
        "papo-peer:1;connection=;nick-casemap=rfc1459;nick=alice;proof=unverified",
        "papo-peer:1;server=irc.example;nick-casemap=rfc1459;nick=;proof=unverified",
        "papo-peer:1;server=irc.example;nick-casemap=rfc1459;nick=ali%GGce;proof=unverified",
        "papo-peer:1;server=irc.example;nick-casemap=rfc1459;nick=alice;proof=account;account-casemap=rfc1459;account=alice;id=",
        "papo-peer:1;server=irc.example;netid=libera;nick-casemap=rfc1459;nick=alice;proof=unverified",
        "papo-peer:1;server=irc.example;bouncer-login=;netid=libera;upstream=irc.libera.chat;nick-casemap=rfc1459;nick=alice;proof=unverified",
        "papo-peer:1;server=irc.example;bouncer-login=alice;netid=;upstream=irc.libera.chat;nick-casemap=rfc1459;nick=alice;proof=unverified",
        "papo-peer:1;server=irc.example;bouncer-login=alice;netid=libera;upstream=;nick-casemap=rfc1459;nick=alice;proof=unverified",
        "papo-peer:1;server=irc.example;nick-casemap=rfc1459;nick=alice;proof=unverified;extra=value",
    ] {
        assert!(
            matches!(value.parse::<PeerKey>(), Err(InvalidPeerKey)),
            "{value}"
        );
    }
}
