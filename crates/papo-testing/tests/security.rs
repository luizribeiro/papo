//! Highest-layer regressions for hostile-server security findings.

use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use papo::{ConnectErrorKind, DisconnectCause, Disconnected, Events, Reconnect, Sasl};
use papo_testing::{Wire, wire};

const PASSWORD: &str = "correct horse battery staple";

fn is_foreign<T>(result: &Result<T, papo::Error>) -> bool {
    matches!(
        result,
        Err(papo::Error::InvalidInput(
            papo::InvalidInput::ForeignReference
        ))
    )
}

async fn next_login_disconnect(events: &mut Events) -> Disconnected {
    loop {
        match events.next().await {
            Some(papo::Event::Disconnected(disconnected)) => return disconnected,
            Some(papo::Event::Connected(_)) => panic!("rejected login unexpectedly connected"),
            Some(_) => {}
            None => panic!("driver stopped without a disconnection event"),
        }
    }
}

async fn next_active_event(events: &mut Events) -> papo::Event {
    match events.next().await {
        Some(papo::Event::Disconnected(disconnected)) => {
            panic!("disconnected while waiting for an event: {disconnected}")
        }
        Some(event) => event,
        None => panic!("driver stopped while waiting for an event"),
    }
}

async fn connected(events: &mut Events) {
    while !matches!(next_active_event(events).await, papo::Event::Connected(_)) {}
}

async fn client_pair(caps: &[&str]) -> (ClientPair, ClientPair) {
    let make = |endpoint, nick| {
        wire(
            papo::Client::builder(endpoint, nick)
                .rate_limit(papo::RateLimit::unlimited())
                .reconnect(Reconnect::never()),
        )
    };
    let (first, first_events, mut first_wire) = make("irc://first.offline.test", "papo-first");
    let (second, second_events, mut second_wire) = make("irc://second.offline.test", "papo-second");
    first_wire.register(caps).await;
    second_wire.register(caps).await;
    (
        (first, first_events, first_wire),
        (second, second_events, second_wire),
    )
}

type ClientPair = (papo::Client, Events, Wire);

fn disconnect_diagnostic(disconnected: &Disconnected) -> String {
    match &disconnected.cause {
        DisconnectCause::Connect(error) => {
            std::error::Error::source(error).map_or_else(|| error.to_string(), ToString::to_string)
        }
        DisconnectCause::Protocol { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

async fn begin_bound_registration(wire: &mut Wire, network: &str) {
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo-bound");
    assert_eq!(wire.next_sent().await, "USER papo-bound 0 * papo-bound");
    wire.server(":soju.example CAP * LS :soju.im/bouncer-networks")
        .await;
    assert_eq!(wire.next_sent().await, "CAP REQ :soju.im/bouncer-networks");
    wire.server(":soju.example CAP * ACK :soju.im/bouncer-networks")
        .await;
    assert_eq!(wire.next_sent().await, format!("BOUNCER BIND {network}"));
    assert_eq!(wire.next_sent().await, "CAP END");
}

fn bound_builder(network: &str) -> papo::Builder {
    papo::Client::builder("irc://soju.offline.test", "papo-bound")
        .rate_limit(papo::RateLimit::unlimited())
        .bouncer_network(papo::NetworkId::from(network))
}

#[tokio::test]
async fn selected_bouncer_network_requires_its_capability() {
    let (_client, mut events, mut wire) = wire(bound_builder("libera-42"));
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo-bound");
    assert_eq!(wire.next_sent().await, "USER papo-bound 0 * papo-bound");
    wire.server(":irc.example CAP * LS :").await;

    let disconnected = next_login_disconnect(&mut events).await;
    assert!(disconnected.reconnect_in.is_none());
    assert!(disconnect_diagnostic(&disconnected).contains("soju.im/bouncer-networks"));
    assert_eq!(wire.next_sent_or_closed().await, None);
}

#[tokio::test]
async fn selected_bouncer_network_requires_matching_isupport() {
    for isupport in [
        "CASEMAPPING=rfc1459",
        "CASEMAPPING=rfc1459 BOUNCER_NETID=oftc-17",
    ] {
        let (_client, mut events, mut wire) = wire(bound_builder("libera-42"));
        begin_bound_registration(&mut wire, "libera-42").await;
        wire.server(":soju.example 001 papo-bound :Welcome to soju")
            .await;
        wire.server(format!(
            ":soju.example 005 papo-bound {isupport} :are supported"
        ))
        .await;
        wire.server(":soju.example 422 papo-bound :MOTD File is missing")
            .await;

        let disconnected = next_login_disconnect(&mut events).await;
        assert!(disconnected.reconnect_in.is_none());
        assert!(disconnect_diagnostic(&disconnected).contains("selected network ID"));
    }
}

#[tokio::test(start_paused = true)]
async fn deleted_bouncer_network_stops_during_reconnect() {
    let (_client, mut events, mut wire) = wire(bound_builder("libera-42"));
    begin_bound_registration(&mut wire, "libera-42").await;
    wire.server(":soju.example 001 papo-bound :Welcome to soju")
        .await;
    wire.server(
        ":soju.example 005 papo-bound CASEMAPPING=rfc1459 BOUNCER_NETID=libera-42 :are supported",
    )
    .await;
    wire.server(":soju.example 422 papo-bound :MOTD File is missing")
        .await;
    while !matches!(
        next_active_event(&mut events).await,
        papo::Event::Connected(_)
    ) {}

    wire.close().await;
    let reconnecting = next_login_disconnect(&mut events).await;
    assert!(reconnecting.is_reconnecting());
    tokio::time::advance(Duration::from_secs(2)).await;
    begin_bound_registration(&mut wire, "libera-42").await;
    wire.server("FAIL BOUNCER INVALID_NETID libera-42 :Invalid network ID")
        .await;

    let stopped = next_login_disconnect(&mut events).await;
    assert!(stopped.reconnect_in.is_none());
    assert!(disconnect_diagnostic(&stopped).contains("rejected the selected network ID"));
    assert_eq!(wire.next_sent_or_closed().await, None);
}

async fn requested_bouncer_caps(builder: papo::Builder) -> BTreeSet<String> {
    let (_client, _events, mut wire) = wire(builder.rate_limit(papo::RateLimit::unlimited()));
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    _ = wire.next_sent().await;
    _ = wire.next_sent().await;
    wire.server(":soju.example CAP * LS :soju.im/bouncer-networks soju.im/bouncer-networks-notify")
        .await;
    wire.next_sent()
        .await
        .strip_prefix("CAP REQ :")
        .unwrap()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn bouncer_notifications_have_one_explicit_source() {
    let notify = "soju.im/bouncer-networks-notify";
    let management = requested_bouncer_caps(
        papo::Client::builder("irc://soju.offline.test", "papo-control")
            .enable(papo::Feature::Bouncer),
    )
    .await;
    assert!(management.contains(notify));

    let bound = requested_bouncer_caps(bound_builder("libera-42")).await;
    assert!(!bound.contains(notify));

    let opted_in =
        requested_bouncer_caps(bound_builder("libera-42").enable(papo::Feature::Bouncer)).await;
    assert!(opted_in.contains(notify));
}

#[tokio::test]
async fn cross_client_values_never_reach_either_wire() {
    let (first, _first_events, mut first_wire) = wire(
        papo::Client::builder("irc://first.offline.test", "papo-first")
            .reconnect(Reconnect::never()),
    );
    let (second, _second_events, mut second_wire) = wire(
        papo::Client::builder("irc://second.offline.test", "papo-second")
            .reconnect(Reconnect::never()),
    );
    first_wire.register(&["message-tags"]).await;
    second_wire
        .register(&[
            "batch",
            "draft/chathistory",
            "labeled-response",
            "message-tags",
            "server-time",
        ])
        .await;
    let history = tokio::spawn({
        let second = second.clone();
        async move { second.history("#papo").latest().await }
    });
    assert_eq!(
        second_wire.next_sent().await,
        "@label=0 CHATHISTORY LATEST #papo * 100"
    );
    for line in [
        "@label=0 :offline.test BATCH +reply labeled-response",
        "@batch=reply;draft/chathistory-end :offline.test BATCH +history chathistory #papo",
        "@batch=history;time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120004 :emily!~e@cloak.example PRIVMSG #papo :release ready",
        "@batch=reply :offline.test BATCH -history",
        ":offline.test BATCH -reply",
    ] {
        second_wire.server(line).await;
    }
    let page = history.await.unwrap().unwrap();
    let message = page.chat_messages().next().unwrap().clone();
    assert_eq!(page.origin(), second.origin());
    assert_eq!(message.origin(), second.origin());

    assert!(is_foreign(&first.reply(&message, "acknowledged").await));
    assert!(is_foreign(
        &first.privmsg(&message.conversation, "acknowledged").await
    ));
    assert!(is_foreign(
        &first
            .history("#papo")
            .before(page.newest_reference().unwrap())
            .await
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), first_wire.next_sent())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(10), second_wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn foreign_monitor_add_never_reaches_either_wire() {
    let ((first, _first_events, mut first_wire), (second, _second_events, mut second_wire)) =
        client_pair(&[]).await;
    second_wire
        .server(":offline.test 005 papo-second MONITOR=100 :are supported")
        .await;
    while second.state().isupport().monitor().is_none() {
        tokio::task::yield_now().await;
    }

    assert!(is_foreign(
        &second
            .monitor()
            .add([first.state().name_rules().nick("alice")])
            .await
    ));
    assert_no_wire(&mut first_wire, &mut second_wire).await;
}

#[tokio::test]
async fn foreign_monitor_remove_never_reaches_either_wire() {
    let ((first, _first_events, mut first_wire), (second, _second_events, mut second_wire)) =
        client_pair(&[]).await;
    second_wire
        .server(":offline.test 005 papo-second MONITOR=100 :are supported")
        .await;
    while second.state().isupport().monitor().is_none() {
        tokio::task::yield_now().await;
    }

    assert!(is_foreign(
        &second
            .monitor()
            .remove([first.state().name_rules().nick("alice")])
            .await
    ));
    assert_no_wire(&mut first_wire, &mut second_wire).await;
}

#[tokio::test]
async fn foreign_account_verification_never_reaches_either_wire() {
    let ((first, _first_events, mut first_wire), (second, _second_events, mut second_wire)) =
        client_pair(&[]).await;

    assert!(is_foreign(
        &second
            .verify_account(first.state().name_rules().account("alice"), "392814")
            .await
    ));
    assert_no_wire(&mut first_wire, &mut second_wire).await;
}

async fn assert_no_wire(first: &mut Wire, second: &mut Wire) {
    for wire in [first, second] {
        assert!(
            tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn foreign_names_cannot_read_another_clients_state() {
    let ((first, _first_events, _first_wire), (second, _second_events, mut second_wire)) =
        client_pair(&["draft/metadata-2", "draft/read-marker"]).await;
    for line in [
        ":papo-second!papo@localhost JOIN #papo",
        ":alice!~alice@cloak.example JOIN #papo",
        ":offline.test 761 papo-second alice avatar * :https://cdn.example.net/alice.png",
        ":offline.test MARKREAD #papo timestamp=2026-10-26T12:00:00.000Z",
    ] {
        second_wire.server(line).await;
    }
    while second.state().metadata("alice").is_none()
        || second.state().read_marker("#papo").is_none()
    {
        tokio::task::yield_now().await;
    }

    let foreign_channel = first.state().name_rules().channel("#papo");
    let foreign_nick = first.state().name_rules().nick("alice");
    assert!(second.state().channel("#papo").is_some());
    assert!(second.state().channel(&foreign_channel).is_none());
    assert!(second.state().user("alice").is_some());
    assert!(second.state().user(&foreign_nick).is_none());
    assert!(second.state().metadata("alice").is_some());
    assert!(second.state().metadata(&foreign_nick).is_none());
    assert!(second.state().read_marker("#papo").is_some());
    assert!(second.state().read_marker(&foreign_channel).is_none());
    let state = second.state();
    let channel = state.channel("#papo").unwrap();
    assert!(channel.member("alice").is_some());
    assert!(channel.member(&foreign_nick).is_none());
}

#[tokio::test]
async fn foreign_return_addresses_cannot_send_typing_or_notices() {
    let (first, mut first_events, mut first_wire) = wire(
        papo::Client::builder("irc://first.offline.test", "papo-first")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    let (_second, mut second_events, mut second_wire) = wire(
        papo::Client::builder("irc://second.offline.test", "papo-second")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    first_wire.register(&["message-tags"]).await;
    second_wire.register(&["message-tags"]).await;
    connected(&mut first_events).await;
    connected(&mut second_events).await;

    second_wire
        .server(":alice!~a@cloak.example PRIVMSG papo-second :release ready")
        .await;
    let papo::Event::Message(message) = next_active_event(&mut second_events).await else {
        panic!("expected a private message")
    };
    let address = message.return_address();

    assert!(is_foreign(
        &first.typing(address.clone(), papo::Typing::Active).await
    ));
    assert!(is_foreign(&first.notice(address, "acknowledged").await));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), first_wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn nickname_handoffs_block_notices_and_ctcp_replies() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .ctcp_replies(papo::CtcpReplies::off())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["message-tags"]).await;
    connected(&mut events).await;
    for line in [
        ":papo-wire!p@localhost JOIN #review",
        ":alice!~a@cloak.example JOIN #review",
        ":mallory!~m@users.example JOIN #review",
    ] {
        wire.server(line).await;
        assert!(matches!(
            next_active_event(&mut events).await,
            papo::Event::Joined(_)
        ));
    }
    wire.server(":alice!~a@cloak.example NOTICE papo-wire :release status")
        .await;
    let papo::Event::Notice(notice) = next_active_event(&mut events).await else {
        panic!("expected a private notice")
    };
    wire.server(":alice!~a@cloak.example PRIVMSG #review :\u{1}VERSION\u{1}")
        .await;
    let papo::Event::CtcpQuery(query) = next_active_event(&mut events).await else {
        panic!("expected a CTCP query")
    };

    wire.server(":alice!~a@cloak.example QUIT :Client Quit")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::Quit(_)
    ));
    wire.server(":mallory!~m@users.example NICK alice").await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::NickChanged(_)
    ));

    assert!(matches!(
        client.notice(notice.return_address(), "classified").await,
        Err(papo::Error::TargetGone)
    ));
    assert!(matches!(
        client.ctcp_reply(&query, "papo 0.1").await,
        Err(papo::Error::TargetGone)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn ctcp_replies_refuse_nick_handoffs() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .ctcp_replies(papo::CtcpReplies::off())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["message-tags"]).await;
    connected(&mut events).await;
    for line in [
        ":papo-wire!p@localhost JOIN #review",
        ":alice!~a@cloak.example JOIN #review",
        ":mallory!~m@users.example JOIN #review",
    ] {
        wire.server(line).await;
        assert!(matches!(
            next_active_event(&mut events).await,
            papo::Event::Joined(_)
        ));
    }
    wire.server(":alice!~a@cloak.example PRIVMSG #review :\u{1}VERSION\u{1}")
        .await;
    let papo::Event::CtcpQuery(channel_query) = next_active_event(&mut events).await else {
        panic!("expected a channel CTCP query")
    };
    wire.server(":alice!~a@cloak.example PRIVMSG papo-wire :\u{1}VERSION\u{1}")
        .await;
    let papo::Event::CtcpQuery(private_query) = next_active_event(&mut events).await else {
        panic!("expected a private CTCP query")
    };

    wire.server(":alice!~a@cloak.example QUIT :Client Quit")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::Quit(_)
    ));
    wire.server(":mallory!~m@users.example NICK alice").await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::NickChanged(_)
    ));

    for query in [&channel_query, &private_query] {
        assert!(matches!(
            client.ctcp_reply(query, "papo 0.1").await,
            Err(papo::Error::TargetGone)
        ));
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn ops_only_return_addresses_survive_notice_and_typing_sends() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["message-tags"]).await;
    connected(&mut events).await;
    wire.server(":offline.test 005 papo-wire STATUSMSG=@+ :are supported")
        .await;
    wire.server(":alice!~a@cloak.example NOTICE @#ops :release status")
        .await;
    let papo::Event::Notice(notice) = next_active_event(&mut events).await else {
        panic!("expected an operators-only notice")
    };
    wire.server("@+typing=paused :alice!~a@cloak.example TAGMSG @#ops")
        .await;
    let papo::Event::TypingChanged(typing) = next_active_event(&mut events).await else {
        panic!("expected an operators-only typing notification")
    };

    client
        .notice(notice.return_address(), "acknowledged")
        .await
        .unwrap();
    assert_eq!(wire.next_sent().await, "NOTICE @#ops acknowledged");
    client
        .typing(typing.return_address(), papo::Typing::Active)
        .await
        .unwrap();
    assert_eq!(wire.next_sent().await, "@+typing=active TAGMSG @#ops");
}

#[derive(Clone, Default)]
struct CaseSensitiveStsStore(Arc<Mutex<HashMap<String, papo::sts::StsPolicy>>>);

impl papo::sts::StsStore for CaseSensitiveStsStore {
    fn get(&self, host: &str) -> Option<papo::sts::StsPolicy> {
        self.0.lock().unwrap().get(host).cloned()
    }

    fn put(&self, policy: papo::sts::StsPolicy) {
        self.0.lock().unwrap().insert(policy.host.clone(), policy);
    }

    fn remove(&self, host: &str) {
        self.0.lock().unwrap().remove(host);
    }
}

async fn start_scram(address: &str) -> (papo::Client, Events, Wire) {
    let (client, events, mut wire) = wire(
        papo::Client::builder(address, "papo-wire")
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
        papo::Client::builder("irc://offline.test", "papo-wire")
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
                && error.to_string().contains(reason)
                && std::error::Error::source(error).is_none()
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
        papo::Client::builder("irc://offline.test", "papo-wire")
            .password(PASSWORD)
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo-wire");
    assert_eq!(wire.next_sent().await, "USER papo-wire 0 * papo-wire");
    wire.server(":ergo.test NOTICE * :*** Looking up your hostname...")
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
    wire.server(":offline.test CAP * LS :sts=port=6697,duration=86400")
        .await;
    assert_eq!(wire.next_sent_or_closed().await, None);
}

#[tokio::test]
async fn server_password_requires_sts_on_plaintext() {
    let (_client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .password(PASSWORD)
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo-wire");
    assert_eq!(wire.next_sent().await, "USER papo-wire 0 * papo-wire");
    wire.server(":offline.test CAP * LS :batch").await;

    let disconnected = next_login_disconnect(&mut events).await;
    assert!(matches!(
        disconnected.cause,
        DisconnectCause::Connect(ref error)
            if error.kind() == ConnectErrorKind::CapabilityUnavailable
    ));
    assert!(disconnect_diagnostic(&disconnected).contains("sts"));
    assert_eq!(wire.next_sent_or_closed().await, None);
}

#[tokio::test]
async fn plaintext_credentials_require_explicit_opt_in() {
    let result = papo::Client::builder("irc://offline.test", "papo-wire")
        .sasl(Sasl::plain("alice", PASSWORD))
        .reconnect(Reconnect::never())
        .connect()
        .await;
    assert!(matches!(result, Err(papo::Error::InvalidInput(_))));
}

#[tokio::test]
async fn plaintext_credential_requests_never_reach_the_wire() {
    let (client, _events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["draft/account-registration", "labeled-response"])
        .await;

    for result in [
        client.oper("admin", PASSWORD).await,
        client
            .verify_account(
                client.state().name_rules().account("alice"),
                "39gvcdg4myvnmdcfhvd6exsv4n",
            )
            .await
            .map(|_| ()),
        client
            .send_raw(
                papo::Line::new("OPER")
                    .param("admin")
                    .param(papo::Secret::new(PASSWORD)),
            )
            .await,
    ] {
        assert!(matches!(
            result,
            Err(papo::Error::InvalidInput(
                papo::InvalidInput::InsecureAuthentication
            ))
        ));
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn raw_mode_response_redacts_the_channel_key() {
    let (client, _events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["labeled-response"]).await;

    let request = tokio::spawn(async move {
        client
            .request_raw(papo::Line::new("MODE").param("#staff"))
            .await
    });
    assert_eq!(wire.next_sent().await, "@label=0 MODE #staff");
    wire.server("@label=0 :offline.test 324 papo-wire #staff +k hunter2")
        .await;

    let lines = request.await.unwrap().unwrap();
    assert!(!format!("{lines:?}").contains("hunter2"));
}

#[tokio::test]
async fn malformed_tls_configuration_fails_before_opening_the_wire() {
    let malformed = b"-----BEGIN CERTIFICATE-----\nnot-base64!\n-----END CERTIFICATE-----\n";
    for scheme in ["irc", "ircs", "ws", "wss"] {
        let endpoint = format!("{scheme}://offline.test");
        assert!(matches!(
            papo::Client::builder(endpoint.as_str(), "papo-wire")
                .tls_ca_pem(malformed)
                .reconnect(Reconnect::never())
                .connect()
                .await,
            Err(papo::Error::InvalidInput(papo::InvalidInput::TlsConfig(message)))
                if message.contains("invalid certificate PEM")
        ));
    }
}

#[tokio::test]
async fn plain_text_rejects_ctcp_delimiters() {
    let (client, _events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["batch"]).await;

    for text in [
        "\x01VERSION\x01".to_owned(),
        format!("{} \x01VERSION\x01", "a".repeat(415)),
        "you said:\n\x01DCC SEND backup.tar 2130706433 5000\x01".to_owned(),
    ] {
        assert!(matches!(
            client.privmsg("#papo", text).await,
            Err(papo::Error::InvalidInput(_))
        ));
    }
}

#[tokio::test]
async fn monitor_rejects_invalid_nicks_without_poisoning_the_wire() {
    let (client, _events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    wire.server(":offline.test 005 papo-wire MONITOR=100 :are supported")
        .await;
    while client.state().isupport().monitor().is_none() {
        tokio::task::yield_now().await;
    }

    assert!(matches!(
        client.monitor().add(["bad\rnick", "alice"]).await,
        Err(papo::Error::InvalidInput(_))
    ));
    client.monitor().add(["bob"]).await.unwrap();
    assert_eq!(wire.next_sent().await, "MONITOR + bob");
}

#[tokio::test]
async fn invalid_registration_fields_fail_before_opening_the_wire() {
    let cases = [
        papo::Client::builder(
            "irc://offline.test",
            papo_testing::nick(papo::Casemapping::default(), "papo\rOPER root password"),
        ),
        papo::Client::builder("irc://offline.test", "papo-client").alt_nicks([papo_testing::nick(
            papo::Casemapping::default(),
            "papo-away\nQUIT",
        )]),
        papo::Client::builder("irc://offline.test", "papo-client").username("papo\0admin"),
        papo::Client::builder("irc://offline.test", "papo-client").realname("Papo\rClient"),
    ];
    for builder in cases {
        assert!(matches!(
            builder.connect().await,
            Err(papo::Error::InvalidInput(_))
        ));
    }
}

#[tokio::test]
async fn invalid_source_nicks_cannot_close_the_client_with_auto_replies() {
    let (_client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .ctcp_replies(
                papo::CtcpReplies::default()
                    .version("papo test build")
                    .rate(papo::RateLimit::unlimited()),
            )
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    while !matches!(
        next_active_event(&mut events).await,
        papo::Event::Connected(_)
    ) {}

    wire.server("::evil!~e@h PRIVMSG papo-wire :\x01VERSION\x01")
        .await;
    assert!(matches!(events.next().await, Some(papo::Event::Other(_))));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), wire.next_sent())
            .await
            .is_err()
    );
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
    let store = CaseSensitiveStsStore::default();
    papo::sts::StsStore::put(
        &store,
        papo::sts::StsPolicy::new(
            "irc.example.net",
            7443,
            SystemTime::now() + Duration::from_secs(3600),
            Duration::from_secs(3600),
        ),
    );
    let (_client, _events, mut wire) = wire(
        papo::Client::builder("irc://IRC.Example.net", "papo-wire")
            .reconnect(Reconnect::never())
            .sts_store(store),
    );

    assert_eq!(wire.endpoint().await, "ircs://IRC.Example.net:7443");
}

#[tokio::test]
async fn oversized_sts_duration_still_persists() {
    let store = CaseSensitiveStsStore::default();
    let inspection = store.clone();
    let (_client, _events, mut wire) = wire(
        papo::Client::builder("ircs://offline.test", "papo-wire")
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

    let websocket_store = CaseSensitiveStsStore::default();
    papo::sts::StsStore::put(
        &websocket_store,
        papo::sts::StsPolicy::new("offline.test", 443, expiry, duration).websocket(true),
    );
    let (_client, _events, mut irc_wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .reconnect(Reconnect::never())
            .sts_store(websocket_store),
    );
    assert_eq!(irc_wire.endpoint().await, "ircs://offline.test");

    let tcp_store = CaseSensitiveStsStore::default();
    papo::sts::StsStore::put(
        &tcp_store,
        papo::sts::StsPolicy::new("offline.test", 6697, expiry, duration),
    );
    let (_client, _events, mut websocket_wire) = wire(
        papo::Client::builder("ws://offline.test/webirc", "papo-wire")
            .reconnect(Reconnect::never())
            .sts_store(tcp_store),
    );
    assert_eq!(websocket_wire.endpoint().await, "wss://offline.test/webirc");

    let (_client, _events, mut live_upgrade) = wire(
        papo::Client::builder("ws://offline.test/webirc", "papo-wire")
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

#[tokio::test(start_paused = true)]
async fn unknown_playback_is_untyped_and_has_no_side_effects() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never())
            .ctcp_replies(papo::CtcpReplies::default().version("papo offline security test")),
    );
    wire.register(&["message-tags", "batch"]).await;
    while !matches!(
        next_active_event(&mut events).await,
        papo::Event::Connected(_)
    ) {}

    for line in [
        ":papo-wire!~p@localhost JOIN #papo",
        ":alice!~a@cloak.example JOIN #papo",
        ":offline.test MODE #papo +m",
        ":alice!~a@cloak.example TOPIC #papo :current topic",
    ] {
        wire.server(line).await;
        events.next().await.expect("live setup event");
    }

    wire.server(":znc.example BATCH +playback znc.in/playback #papo")
        .await;
    for line in [
        "@batch=playback :bob!~b@remote.example JOIN #papo",
        "@batch=playback :alice!~a@cloak.example PART #papo :old part",
        "@batch=playback :operator!~o@services.example KICK #papo papo-wire :old kick",
        "@batch=playback :operator!~o@services.example MODE #papo -m",
        "@batch=playback :operator!~o@services.example TOPIC #papo :stale topic",
        "@batch=playback :alice!~a@cloak.example NICK alice-away",
        "@batch=playback :alice!~a@cloak.example PRIVMSG papo-wire :\u{1}VERSION\u{1}",
    ] {
        wire.server(line).await;
        assert!(matches!(events.next().await, Some(papo::Event::Other(_))));
    }
    wire.server(":znc.example BATCH -playback").await;

    let state = client.state();
    let channel = state.channel("#papo").expect("own join remains cached");
    assert_eq!(channel.topic(), Some("current topic"));
    assert!(channel.member("bob").is_none());
    assert!(channel.member("alice-away").is_none());
    assert!(channel.member("alice").is_some());
    assert!(
        tokio::time::timeout(Duration::from_millis(1), wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn typing_status_change_preserves_the_latest_audience() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["message-tags"]).await;
    while !matches!(
        next_active_event(&mut events).await,
        papo::Event::Connected(_)
    ) {}
    wire.server(":offline.test 005 papo-wire STATUSMSG=@+ :are supported by this server")
        .await;

    wire.server("@+typing=active :alice!~a@cloak.example TAGMSG #staff")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::TypingChanged(_)
    ));
    wire.server("@+typing=active :alice!~a@cloak.example TAGMSG @#staff")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::TypingChanged(_)
    ));

    let conversation = client.state().name_rules().conversation("#staff").unwrap();
    let state = client.state();
    let typer = state.typing(&conversation).next().unwrap();
    assert_eq!(typer.return_address(), "@#staff");
}

#[tokio::test]
async fn channel_keys_are_redacted_across_state_and_events() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    while !matches!(
        next_active_event(&mut events).await,
        papo::Event::Connected(_)
    ) {}
    wire.server(
        ":offline.test 005 papo-wire CHANMODES=Ibe,k,fl,imnpst :are supported by this server",
    )
    .await;
    wire.server(":papo-wire!~p@localhost JOIN #staff").await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::Joined(_)
    ));

    wire.server(":offline.test 324 papo-wire #staff +k snapshot-key-2026")
        .await;
    wire.server(":offline.test 777 papo-wire :snapshot applied")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::Other(line) if line.numeric() == Some(777)
    ));
    let state = client.state();
    let channel = state.channel("#staff").unwrap();
    assert_eq!(
        channel.key().map(papo::Secret::expose),
        Some("snapshot-key-2026")
    );
    assert!(!format!("{state:?}").contains("snapshot-key-2026"));

    wire.server(":alice!~a@cloak.example MODE #staff +k hunter2")
        .await;
    let mode = next_active_event(&mut events).await;
    assert!(!format!("{mode:?}").contains("hunter2"));
    assert!(matches!(mode, papo::Event::ModeChanged(change)
        if matches!(&change.changes[0].arg, Some(papo::ModeArg::Key(key)) if key.expose() == "hunter2")));

    wire.server(":offline.test 324 papo-wire #unjoined +k raw-key-2026")
        .await;
    let raw = next_active_event(&mut events).await;
    assert!(matches!(&raw, papo::Event::Other(line) if line.numeric() == Some(324)));
    assert!(!format!("{raw:?}").contains("raw-key-2026"));

    wire.server(":offline.test BATCH +opaque vendor.example/archive")
        .await;
    wire.server("@batch=opaque :alice!~a@cloak.example MODE #staff +k archived-key-2026")
        .await;
    let raw = next_active_event(&mut events).await;
    assert!(matches!(&raw, papo::Event::Other(line) if line.command() == "MODE"));
    assert!(!format!("{raw:?}").contains("archived-key-2026"));
    wire.server(":offline.test BATCH -opaque").await;
}

#[tokio::test(start_paused = true)]
async fn expired_labeled_history_is_not_delivered_twice() {
    let (_client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "message-tags",
        "batch",
        "labeled-response",
        "draft/chathistory",
    ])
    .await;
    while !matches!(
        next_active_event(&mut events).await,
        papo::Event::Connected(_)
    ) {}

    for line in [
        "@label=expired :offline.test BATCH +reply-42 labeled-response",
        "@batch=reply-42 :offline.test BATCH +history-42 chathistory #papo",
        "@batch=history-42;time=2026-10-15T12:00:01.000Z;msgid=RKi9Cw :alice!~a@cloak.example PRIVMSG #papo :earlier deployment",
        "@batch=reply-42 :offline.test BATCH -history-42",
        ":offline.test BATCH -reply-42",
    ] {
        wire.server(line).await;
    }

    let page = events
        .next()
        .await
        .and_then(|event| event.as_history().cloned())
        .expect("expired reply still produces one history page");
    assert_eq!(
        page.chat_messages()
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>(),
        ["earlier deployment"]
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(1), events.next())
            .await
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn late_labeled_reply_is_delivered_as_a_live_message() {
    let (_client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["message-tags", "batch", "labeled-response"])
        .await;
    while !matches!(
        next_active_event(&mut events).await,
        papo::Event::Connected(_)
    ) {}

    wire.server("@label=expired :offline.test BATCH +reply-42 labeled-response")
        .await;
    wire.server("@batch=reply-42 :offline.test NOTICE papo-wire :request completed late")
        .await;
    wire.server(":offline.test BATCH -reply-42").await;

    assert!(matches!(
        events.next().await,
        Some(papo::Event::ServerNotice(notice)) if notice.text == "request completed late"
    ));
}

#[tokio::test(start_paused = true)]
async fn oversized_multiline_fragments_are_live_messages() {
    let (_client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "message-tags",
        "batch",
        "draft/multiline=max-bytes=10,max-lines=5",
    ])
    .await;
    while !matches!(
        next_active_event(&mut events).await,
        papo::Event::Connected(_)
    ) {}

    wire.server(":offline.test BATCH +multiline draft/multiline #papo")
        .await;
    wire.server("@batch=multiline :alice!~a@cloak.example PRIVMSG #papo :123456")
        .await;
    wire.server("@batch=multiline :alice!~a@cloak.example PRIVMSG #papo :789012")
        .await;

    for expected in ["123456", "789012"] {
        assert!(matches!(events.next().await,
            Some(papo::Event::Message(message)) if message.text == expected));
    }
    assert!(matches!(
        events.next().await,
        Some(papo::Event::Warning(papo::event::Warning::BatchAbandoned {
            reason: papo::event::BatchAbandonReason::ByteLimit,
            ..
        }))
    ));
    wire.server(":offline.test BATCH -multiline").await;
}
