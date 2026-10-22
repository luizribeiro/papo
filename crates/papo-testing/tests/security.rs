//! Highest-layer regressions for hostile-server security findings.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use papo::{ConnectErrorKind, DisconnectCause, Disconnected, Events, Reconnect, Sasl};
use papo_testing::{Wire, wire};

const PASSWORD: &str = "correct horse battery staple";

#[derive(Clone, Default)]
struct CaseSensitiveStsStore(Arc<Mutex<HashMap<String, papo::sts::StsPolicy>>>);

impl papo::sts::StsStore for CaseSensitiveStsStore {
    fn get(&self, host: &str) -> Option<papo::sts::StsPolicy> {
        self.0.lock().unwrap().get(host).cloned()
    }

    fn put(&mut self, policy: papo::sts::StsPolicy) {
        self.0.lock().unwrap().insert(policy.host.clone(), policy);
    }

    fn remove(&mut self, host: &str) {
        self.0.lock().unwrap().remove(host);
    }
}

async fn start_scram(address: &str) -> (papo::Client, Events, Wire) {
    let (client, events, mut wire) = wire(
        papo::Client::builder(address)
            .nick("papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never())
            .sasl(Sasl::any("alice", PASSWORD))
            .allow_insecure_auth(true),
    );
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo-wire");
    assert_eq!(wire.next_sent().await, "USER papo-wire 0 * papo-wire");
    wire.server(":offline.test CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
        .await;
    assert_eq!(wire.next_sent().await, "CAP REQ :sasl");
    wire.server(":offline.test CAP * ACK :sasl").await;
    assert_eq!(wire.next_sent().await, "AUTHENTICATE SCRAM-SHA-256");
    (client, events, wire)
}

async fn start_cap_negotiation() -> (papo::Client, Events, Wire) {
    let (client, events, mut wire) = wire(
        papo::Client::builder("irc://offline.test")
            .nick("papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never())
            .sasl(Sasl::any("alice", PASSWORD))
            .allow_insecure_auth(true),
    );
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo-wire");
    assert_eq!(wire.next_sent().await, "USER papo-wire 0 * papo-wire");
    wire.server(":offline.test CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
        .await;
    assert_eq!(wire.next_sent().await, "CAP REQ :sasl");
    (client, events, wire)
}

async fn send_bad_server_proof(wire: &mut Wire) {
    wire.server("AUTHENTICATE +").await;
    let client_first = wire.next_sent().await;
    let encoded = client_first
        .strip_prefix("AUTHENTICATE ")
        .expect("client did not send a SCRAM client-first message");
    let decoded = STANDARD
        .decode(encoded)
        .expect("client-first message was not base64");
    let decoded = String::from_utf8(decoded).expect("client-first message was not UTF-8");
    let nonce = decoded
        .strip_prefix("n,,n=alice,r=")
        .expect("client-first message had an unexpected shape");
    let server_first = format!("r={nonce}offline-server,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096");
    wire.server(format!("AUTHENTICATE {}", STANDARD.encode(server_first)))
        .await;
    assert!(
        wire.next_sent().await.starts_with("AUTHENTICATE "),
        "client did not send a SCRAM client-final message"
    );
    let bad_proof = format!("v={}", STANDARD.encode([0_u8; 32]));
    wire.server(format!("AUTHENTICATE {}", STANDARD.encode(bad_proof)))
        .await;
    assert_eq!(wire.next_sent().await, "AUTHENTICATE *");
}

async fn send_excessive_iteration_count(wire: &mut Wire) {
    wire.server("AUTHENTICATE +").await;
    let client_first = wire.next_sent().await;
    let encoded = client_first.strip_prefix("AUTHENTICATE ").unwrap();
    let decoded = String::from_utf8(STANDARD.decode(encoded).unwrap()).unwrap();
    let nonce = decoded.strip_prefix("n,,n=alice,r=").unwrap();
    let challenge = format!("r={nonce}offline-server,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4294967295");
    wire.server(format!("AUTHENTICATE {}", STANDARD.encode(challenge)))
        .await;
}

async fn sasl_disconnect(events: &mut Events) -> Disconnected {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match events.next().await {
                Some(papo::Event::Disconnected(disconnected)) => return disconnected,
                Some(_) => {}
                None => panic!("driver stopped without a disconnection event"),
            }
        }
    })
    .await
    .expect("hostile server input did not fail SASL")
}

fn assert_sasl_failure(disconnected: &Disconnected, reason: &str) {
    assert!(
        matches!(
        &disconnected.cause,
        DisconnectCause::Connect(error)
            if error.kind() == ConnectErrorKind::Sasl
                && std::error::Error::source(error)
                    .is_some_and(|source| source.to_string().contains(reason))
        ),
        "unexpected disconnection: {disconnected:?}"
    );
    assert!(disconnected.reconnect_in.is_none());
}

#[tokio::test(start_paused = true)]
async fn scram_success_requires_verified_server_proof() {
    for numeric in [903, 907] {
        let (_client, mut events, mut wire) = start_scram("irc://offline.test").await;
        wire.server(format!(
            ":offline.test {numeric} papo-wire :SASL authentication successful"
        ))
        .await;
        let disconnected = sasl_disconnect(&mut events).await;
        assert_sasl_failure(&disconnected, "verified server proof");
    }

    for numeric in [903, 907] {
        let (_client, mut events, mut wire) = start_cap_negotiation().await;
        wire.server(format!(
            ":offline.test {numeric} papo-wire :SASL authentication successful"
        ))
        .await;
        wire.server(":offline.test CAP * NAK :sasl").await;
        let disconnected = sasl_disconnect(&mut events).await;
        assert!(matches!(
            &disconnected.cause,
            DisconnectCause::Connect(error)
                if error.kind() == ConnectErrorKind::SaslUnavailable
        ));
    }

    let (_client, mut events, mut wire) = start_scram("irc://offline.test").await;
    send_bad_server_proof(&mut wire).await;
    wire.server(":offline.test 903 papo-wire :SASL authentication successful")
        .await;
    let disconnected = sasl_disconnect(&mut events).await;
    assert_sasl_failure(&disconnected, "ServerSignatureMismatch");
}

#[tokio::test(start_paused = true)]
async fn scram_never_falls_back_to_plain_unsafely() {
    for advertise_plain in [false, true] {
        let (_client, mut events, mut wire) = start_scram("irc://offline.test").await;
        if advertise_plain {
            wire.server(":offline.test 908 papo-wire PLAIN :available SASL mechanisms")
                .await;
        }
        wire.server(":offline.test 904 papo-wire :SASL authentication failed")
            .await;
        let disconnected = sasl_disconnect(&mut events).await;
        assert_sasl_failure(&disconnected, "PLAIN fallback");
    }

    let (_client, _events, mut wire) = start_scram("ircs://offline.test").await;
    wire.server(":offline.test 904 papo-wire :SASL authentication failed")
        .await;
    assert_eq!(wire.next_sent().await, "AUTHENTICATE PLAIN");

    let (_client, mut events, mut wire) = start_scram("ircs://offline.test").await;
    wire.server("AUTHENTICATE +").await;
    assert!(wire.next_sent().await.starts_with("AUTHENTICATE "));
    wire.server(":offline.test 904 papo-wire :SASL authentication failed")
        .await;
    let disconnected = sasl_disconnect(&mut events).await;
    assert_sasl_failure(&disconnected, "PLAIN fallback");

    let (_client, mut events, mut wire) = start_scram("ircs://offline.test").await;
    send_bad_server_proof(&mut wire).await;
    wire.server(":offline.test 906 papo-wire :SASL authentication aborted")
        .await;
    let disconnected = sasl_disconnect(&mut events).await;
    assert_sasl_failure(&disconnected, "ServerSignatureMismatch");
}

#[tokio::test]
async fn server_password_waits_for_sts() {
    let (_client, _events, mut wire) = wire(
        papo::Client::builder("irc://offline.test")
            .nick("papo-wire")
            .password(PASSWORD)
            .allow_insecure_auth(true)
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo-wire");
    assert_eq!(wire.next_sent().await, "USER papo-wire 0 * papo-wire");
    wire.server(":offline.test CAP * LS :sts=port=6697,duration=86400")
        .await;
    assert_eq!(wire.next_sent_or_closed().await, None);
}

#[tokio::test]
async fn plaintext_credentials_require_explicit_opt_in() {
    let result = papo::Client::builder("irc://offline.test")
        .nick("papo-wire")
        .sasl(Sasl::plain("alice", PASSWORD))
        .reconnect(Reconnect::never())
        .connect()
        .await;
    assert!(matches!(result, Err(papo::Error::InvalidInput(_))));
}

#[tokio::test]
async fn scram_rejects_excessive_iterations_promptly() {
    let (_client, mut events, mut wire) = start_scram("ircs://offline.test").await;
    send_excessive_iteration_count(&mut wire).await;
    assert_eq!(wire.next_sent().await, "AUTHENTICATE *");
    wire.server(":offline.test 906 papo-wire :SASL authentication aborted")
        .await;
    let disconnected = sasl_disconnect(&mut events).await;
    assert_sasl_failure(&disconnected, "IterationCountTooHigh");
}

#[tokio::test]
async fn stored_sts_matches_a_normalized_hostname() {
    let mut store = CaseSensitiveStsStore::default();
    papo::sts::StsStore::put(
        &mut store,
        papo::sts::StsPolicy::new(
            "irc.example.net",
            7443,
            SystemTime::now() + Duration::from_secs(3600),
            Duration::from_secs(3600),
        ),
    );
    let (_client, _events, mut wire) = wire(
        papo::Client::builder("irc://IRC.Example.net.")
            .nick("papo-wire")
            .reconnect(Reconnect::never())
            .sts_store(store),
    );

    assert_eq!(wire.endpoint().await, "ircs://IRC.Example.net.:7443");
}

#[tokio::test]
async fn oversized_sts_duration_still_persists() {
    let store = CaseSensitiveStsStore::default();
    let inspection = store.clone();
    let (_client, _events, mut wire) = wire(
        papo::Client::builder("ircs://offline.test")
            .nick("papo-wire")
            .reconnect(Reconnect::never())
            .sts_store(store),
    );
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo-wire");
    assert_eq!(wire.next_sent().await, "USER papo-wire 0 * papo-wire");
    wire.server(format!(":offline.test CAP * LS :sts=duration={}", u64::MAX))
        .await;

    let policy = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(policy) = papo::sts::StsStore::get(&inspection, "offline.test") {
                return policy;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("oversized STS policy was not persisted");
    let in_one_hundred_years = SystemTime::now()
        .checked_add(Duration::from_hours(876_000))
        .unwrap();
    assert!(policy.is_active_at(in_one_hundred_years));
}

#[tokio::test]
async fn stored_sts_ports_stay_with_their_transport() {
    let duration = Duration::from_secs(3600);
    let expiry = SystemTime::now() + duration;

    let mut websocket_store = CaseSensitiveStsStore::default();
    papo::sts::StsStore::put(
        &mut websocket_store,
        papo::sts::StsPolicy::new("offline.test", 443, expiry, duration).websocket(true),
    );
    let (_client, _events, mut irc_wire) = wire(
        papo::Client::builder("irc://offline.test")
            .nick("papo-wire")
            .reconnect(Reconnect::never())
            .sts_store(websocket_store),
    );
    assert_eq!(irc_wire.endpoint().await, "ircs://offline.test");

    let mut tcp_store = CaseSensitiveStsStore::default();
    papo::sts::StsStore::put(
        &mut tcp_store,
        papo::sts::StsPolicy::new("offline.test", 6697, expiry, duration),
    );
    let (_client, _events, mut websocket_wire) = wire(
        papo::Client::builder("ws://offline.test/webirc")
            .nick("papo-wire")
            .reconnect(Reconnect::never())
            .sts_store(tcp_store),
    );
    assert_eq!(websocket_wire.endpoint().await, "wss://offline.test/webirc");

    let (_client, _events, mut live_upgrade) = wire(
        papo::Client::builder("ws://offline.test/webirc")
            .nick("papo-wire")
            .rate_limit(papo::RateLimit::unlimited()),
    );
    assert_eq!(live_upgrade.endpoint().await, "ws://offline.test/webirc");
    assert_eq!(live_upgrade.next_sent().await, "CAP LS 302");
    assert_eq!(live_upgrade.next_sent().await, "NICK papo-wire");
    assert_eq!(
        live_upgrade.next_sent().await,
        "USER papo-wire 0 * papo-wire"
    );
    live_upgrade
        .server(":offline.test CAP * LS :sts=port=6697")
        .await;
    assert_eq!(live_upgrade.next_sent_or_closed().await, None);
    live_upgrade.close().await;
    assert_eq!(live_upgrade.endpoint().await, "wss://offline.test/webirc");
}
