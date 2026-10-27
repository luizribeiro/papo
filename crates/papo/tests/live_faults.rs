//! Client recovery and bounded shutdown against injected local faults.
#![cfg(not(target_family = "wasm"))]

mod support;

use std::time::{Duration, Instant};

use papo::{Client, ConnectErrorKind, DisconnectCause, Endpoint, Event, Line, Reconnect, Sasl};
use papo_test_server::{
    OPER_NAME, OPER_PASSWORD, SolanumServer, TestServer, register_account_at, unique,
};
use support::within;

#[tokio::test(flavor = "current_thread")]
async fn stale_credentials_end_reconnect_with_a_connect_cause() {
    within(async {
        let server = TestServer::start_persistent().unwrap();
        let account = unique("papo-credential");
        let first = "q7Hx-2mZp_9vLcT4wRfB";
        let replacement = "r8Ky-3nAq_6wMdP5xTgC";
        register_account_at(server.plaintext_addr(), &account, first).unwrap();
        let (client, mut events) = Client::builder(
            Endpoint::plaintext("localhost", server.plaintext_addr().port()).unwrap(),
            support::nick(&account),
        )
        .allow_insecure_auth(true)
        .sasl(Sasl::plain(&account, first))
        .reconnect(
            Reconnect::exponential()
                .max_delay(Duration::from_millis(50))
                .max_attempts(2),
        )
        .connect()
        .await
        .unwrap();
        wait_for(&mut events, |event| matches!(event, Event::Connected(_))).await;

        server
            .change_account_password(&account, first, replacement)
            .unwrap();
        client.reconnect().await.unwrap();
        let disconnected = wait_for(&mut events, |event| {
            matches!(
                event,
                Event::Disconnected(disconnected)
                    if matches!(disconnected.cause, DisconnectCause::Connect(_))
            )
        })
        .await;
        let Event::Disconnected(disconnected) = disconnected else {
            unreachable!()
        };
        assert!(matches!(disconnected.cause, DisconnectCause::Connect(_)));
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn solanum_kline_after_welcome_stops_reconnecting() {
    within(async {
        let server = SolanumServer::start().unwrap();
        let endpoint = Endpoint::plaintext("localhost", server.endpoints()[0].port()).unwrap();
        let (operator, mut operator_events) = Client::builder(endpoint.clone(), "papo-oper")
            .allow_insecure_auth(true)
            .connect()
            .await
            .unwrap();
        let (_victim, mut victim_events) = Client::builder(endpoint, "papo-kline")
            .username("klineuser")
            .reconnect(Reconnect::exponential().max_delay(Duration::from_millis(10)))
            .connect()
            .await
            .unwrap();
        wait_for(&mut operator_events, |event| {
            matches!(event, Event::Connected(_))
        })
        .await;
        wait_for(&mut victim_events, |event| {
            matches!(event, Event::Connected(_))
        })
        .await;
        operator.oper(OPER_NAME, OPER_PASSWORD).await.unwrap();

        operator
            .send_raw(Line::new("KLINE").with_params([
                "1",
                "*klineuser@*",
                "post-welcome retry test",
            ]))
            .await
            .unwrap();
        let disconnected = wait_for(&mut victim_events, |event| {
            matches!(event, Event::Disconnected(_))
        })
        .await;
        let Event::Disconnected(disconnected) = disconnected else {
            unreachable!()
        };
        assert!(matches!(
            disconnected.cause,
            DisconnectCause::Connect(ref error) if error.kind() == ConnectErrorKind::Banned
        ));
        assert!(!disconnected.is_reconnecting());
        assert!(victim_events.next().await.is_none());
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn quit_returns_within_its_bound_when_the_peer_stops_reading() {
    within(async {
        let server = TestServer::start_default().unwrap();
        let proxy = server.fault_proxy().unwrap();
        let nick = unique("papo-black-hole");
        let (client, mut events) = Client::builder(
            Endpoint::plaintext("localhost", proxy.address().port()).unwrap(),
            support::nick(&nick),
        )
        .connect()
        .await
        .unwrap();
        wait_for(&mut events, |event| matches!(event, Event::Connected(_))).await;
        proxy.black_hole().unwrap();

        let started = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(11),
            client.quit("bounded shutdown with a stalled peer"),
        )
        .await;
        assert!(result.is_ok(), "quit exceeded eleven seconds");
        assert!(started.elapsed() <= Duration::from_secs(11));
    })
    .await;
}

async fn wait_for(events: &mut papo::Events, accepted: impl Fn(&Event) -> bool) -> Event {
    loop {
        let event = events.next().await.expect("driver stopped unexpectedly");
        if accepted(&event) {
            return event;
        }
    }
}
