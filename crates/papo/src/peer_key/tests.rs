use std::{collections::hash_map::DefaultHasher, hash::Hash as _, hash::Hasher as _};

use super::{InvalidPeerKey, PeerKey, PeerKeyCheck, PeerKeyMismatch, PeerScope};

use crate::{Account, Casemapping};

fn parse(fields: &str) -> PeerKey {
    format!("papo-peer:1;{fields}").parse().unwrap()
}

fn network_key(server: &str, nick: &str, proof: &str) -> PeerKey {
    parse(&format!(
        "server={server};nick-casemap=rfc1459;nick={nick};{proof}"
    ))
}

fn hash(key: &PeerKey) -> u64 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn peer_key_eq_and_hash_ignore_proof_upgrade() {
    let unverified = network_key("irc.example", "alice", "proof=unverified");
    let account = network_key(
        "irc.example",
        "alice",
        "proof=account;account-casemap=rfc1459;account=alice",
    );
    let account_id = network_key(
        "irc.example",
        "alice",
        "proof=account;account-casemap=rfc1459;account=alice;id=account-42",
    );

    assert_eq!(unverified, account);
    assert_eq!(account, account_id);
    assert_eq!(hash(&unverified), hash(&account_id));
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
fn peer_key_changes_for_network_or_folded_nick() {
    let alice = network_key("irc.example", "Alice", "proof=unverified");
    let folded_alice = network_key("irc.example", "alice", "proof=unverified");
    let ascii_alice = parse("server=irc.example;nick-casemap=ascii;nick=alice;proof=unverified");
    let other_network = network_key("chat.example", "alice", "proof=unverified");
    let bob = network_key("irc.example", "bob", "proof=unverified");

    assert_eq!(alice, folded_alice);
    assert_eq!(alice, ascii_alice);
    assert_ne!(alice, other_network);
    assert_ne!(alice, bob);
    assert_ne!(hash(&alice), hash(&other_network));
    assert_ne!(hash(&alice), hash(&bob));
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
fn account_proof_checks_every_upgrade_and_mismatch() {
    let name = network_key(
        "irc.example",
        "alice",
        "proof=account;account-casemap=rfc1459;account=alice",
    );
    let identified = network_key(
        "irc.example",
        "alice",
        "proof=account;account-casemap=rfc1459;account=alice;id=account-42",
    );
    assert!(matches!(name.check(&name), PeerKeyCheck::Match));
    assert!(matches!(
        name.check(&identified),
        PeerKeyCheck::Upgrade(key) if key.to_string() == identified.to_string()
    ));
    assert!(matches!(
        identified.check(&name),
        PeerKeyCheck::Mismatch(PeerKeyMismatch::CannotVerify)
    ));
    for current in [
        network_key(
            "irc.example",
            "alice",
            "proof=account;account-casemap=rfc1459;account=bob",
        ),
        network_key(
            "irc.example",
            "alice",
            "proof=account;account-casemap=ascii;account=alice",
        ),
        network_key(
            "irc.example",
            "bob",
            "proof=account;account-casemap=rfc1459;account=alice",
        ),
    ] {
        assert!(matches!(
            name.check(&current),
            PeerKeyCheck::Mismatch(PeerKeyMismatch::DifferentAccount)
        ));
    }
    let replacement = network_key(
        "irc.example",
        "alice",
        "proof=account;account-casemap=rfc1459;account=alice;id=account-99",
    );
    assert!(matches!(
        identified.check(&replacement),
        PeerKeyCheck::Mismatch(PeerKeyMismatch::DifferentAccount)
    ));
}

#[test]
fn mismatched_threads_need_durable_proof_on_both_sides() {
    let account = network_key(
        "irc.ergo.chat",
        "%5BAlice%5D",
        "proof=account;account-casemap=rfc1459;account=alice",
    );
    let continuity = network_key("irc.ergo.chat", "bob", "proof=unverified");

    assert!(matches!(
        account.check(&continuity),
        PeerKeyCheck::Mismatch(PeerKeyMismatch::CannotVerify)
    ));
}

#[test]
fn peer_keys_compare_by_scope_and_canonical_account() {
    let account = Account::LoggedIn(Casemapping::Rfc1459.account("Alice[Ops]"));
    let nick = Casemapping::Rfc1459.nick("Alice");
    let first =
        PeerKey::historical(Some(&PeerScope::server("irc.example")), &nick, &account).unwrap();
    let same =
        PeerKey::historical(Some(&PeerScope::server("irc.example")), &nick, &account).unwrap();
    let other =
        PeerKey::historical(Some(&PeerScope::server("chat.example")), &nick, &account).unwrap();

    assert!(first.is_verified());
    assert!(matches!(first.check(&same), PeerKeyCheck::Match));
    assert!(matches!(
        first.check(&other),
        PeerKeyCheck::Mismatch(PeerKeyMismatch::DifferentAccount)
    ));
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
