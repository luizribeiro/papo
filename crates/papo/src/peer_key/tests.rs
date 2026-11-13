use std::{collections::hash_map::DefaultHasher, hash::Hash as _, hash::Hasher as _};

use super::{NotSamePerson, PeerKey, PeerKeyCheck, PeerKeyMismatch, PeerProof, PeerScope};

use crate::{Account, Casemapping};
use papo_core::{Config, Session};

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

fn joined_alice() -> Session {
    let now = crate::rt::now_mono();
    let mut session = Session::new(Config::default().nick("papo"));
    session.handle_line(now, b":papo!papo@localhost JOIN #release");
    session.handle_line(
        now,
        b":[Alice]!alice@staff.ergo.chat JOIN #release * :Alice Example",
    );
    session
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
fn not_same_person_explains_the_nickname_handoff() {
    let current = network_key("irc.example", "alice", "proof=unverified");
    let mismatch = NotSamePerson::new(PeerKeyMismatch::CannotVerify, current);

    assert_eq!(
        mismatch.to_string(),
        "the current nickname holder is not the saved person"
    );
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
fn account_proof_does_not_retain_an_unknown_continuity_token() {
    let session = Session::new(Config::default().nick("papo"));
    let nick = Casemapping::Rfc1459.nick("[Alice]");
    let account = Account::LoggedIn(Casemapping::Rfc1459.account("alice"));
    let key = PeerKey::live(
        Some(&PeerScope::server("irc.ergo.chat")),
        &nick,
        &account,
        None,
        session.state().__peer_token_for("[Alice]"),
    )
    .unwrap();

    assert!(matches!(key.proof, PeerProof::Account(_, None)));
}

#[test]
fn unknown_continuity_has_no_live_peer_key() {
    let session = Session::new(Config::default().nick("papo"));
    let nick = Casemapping::Rfc1459.nick("[Alice]");

    assert!(
        PeerKey::live(
            Some(&PeerScope::server("irc.ergo.chat")),
            &nick,
            &Account::Unknown,
            None,
            session.state().__peer_token_for("[Alice]"),
        )
        .is_none()
    );
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
fn stale_continuity_does_not_match_a_reused_nickname() {
    let now = crate::rt::now_mono();
    let mut session = joined_alice();
    let scope = PeerScope::server("irc.ergo.chat");
    let nick = Casemapping::Rfc1459.nick("[Alice]");
    let stored = PeerKey::live(
        Some(&scope),
        &nick,
        &Account::Unknown,
        None,
        session.state().__peer_token_for("[Alice]"),
    )
    .unwrap();
    session.handle_line(now, b":[Alice]!alice@staff.ergo.chat QUIT :Changing hosts");
    session.handle_line(
        now,
        b":[Alice]!stranger@users.ergo.chat JOIN #release * :Mallory Example",
    );
    let current = PeerKey::live(
        Some(&scope),
        &nick,
        &Account::Unknown,
        None,
        session.state().__peer_token_for("[Alice]"),
    )
    .unwrap();

    assert!(matches!(
        stored.check(&current),
        PeerKeyCheck::Mismatch(PeerKeyMismatch::CannotVerify)
    ));
}

#[test]
fn stale_continuity_does_not_upgrade_a_strangers_account() {
    let now = crate::rt::now_mono();
    let mut session = joined_alice();
    let scope = PeerScope::server("irc.ergo.chat");
    let nick = Casemapping::Rfc1459.nick("[Alice]");
    let stored = PeerKey::live(
        Some(&scope),
        &nick,
        &Account::Unknown,
        None,
        session.state().__peer_token_for("[Alice]"),
    )
    .unwrap();
    session.handle_line(now, b":[Alice]!alice@staff.ergo.chat QUIT :Changing hosts");
    session.handle_line(
        now,
        b":[Alice]!stranger@users.ergo.chat JOIN #release mallory :Mallory Example",
    );
    let current = PeerKey::live(
        Some(&scope),
        &nick,
        &Account::LoggedIn(Casemapping::Rfc1459.account("mallory")),
        None,
        session.state().__peer_token_for("[Alice]"),
    )
    .unwrap();

    assert!(matches!(
        stored.check(&current),
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
fn unverified_history_has_no_peer_key() {
    assert!(
        PeerKey::historical(
            None,
            &Casemapping::Rfc1459.nick("guest-4821"),
            &Account::LoggedOut
        )
        .is_none()
    );
}
