//! Account APIs against the runner-managed Ergo server.
#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

mod support;

use support::within;

#[cfg(not(target_family = "wasm"))]
use std::net::SocketAddr;
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

#[cfg(not(target_family = "wasm"))]
use papo::Reconnect;
use papo::{AccountRegistration, Client, Endpoint, Event, NewAccount, Sasl};
#[cfg(not(target_family = "wasm"))]
use papo_core::PersistenceSetting;
#[cfg(not(target_family = "wasm"))]
use papo_test_server::register_account;
use papo_test_server::{from_env, unique};
#[cfg(not(target_family = "wasm"))]
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
};

const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

#[tokio::test(flavor = "current_thread")]
async fn registers_then_logs_in_with_sasl() {
    within(async {
        let server = from_env();
        let address = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        );
        let account = unique("papo-account");
        let (registrar, mut registrar_events) = Client::builder(address.clone())
            .nick(&account)
            .request_caps(["draft/account-registration", "labeled-response"])
            .connect()
            .await
            .unwrap();
        wait_registered(&mut registrar_events, &account).await;
        assert!(matches!(
            registrar
                .register_account(NewAccount::new(PASSWORD).account(&account))
                .await
                .unwrap(),
            AccountRegistration::Registered { account: registered } if registered == account
        ));
        registrar.quit(Some("account created")).await.unwrap();

        let (authenticated, mut authenticated_events) = Client::builder(address)
            .nick(&account)
            .sasl(Sasl::plain(&account, PASSWORD))
            .request_caps(["sasl"])
            .connect()
            .await
            .unwrap();
        wait_registered(&mut authenticated_events, &account).await;
        assert_eq!(
            authenticated.state(|state| state.account().map(str::to_owned)),
            Some(account.clone())
        );
        authenticated
            .quit(Some("SASL login verified"))
            .await
            .unwrap();
    })
    .await;
}

#[cfg(not(target_family = "wasm"))]
#[tokio::test(flavor = "current_thread")]
async fn persistent_server_restores_channels_without_a_client_join() {
    within(async {
        let server = from_env();
        let (proxy, disconnect) = disconnecting_proxy(server.plaintext_addr()).await;
        let address = Endpoint::plaintext(proxy.ip().to_string(), proxy.port());
        let account = unique("papo-persistent");
        let channel = format!("#{}", unique("papo-persistent"));
        register_account(&server, &account, PASSWORD).unwrap();
        let (client, mut events) = Client::builder(address)
            .nick(&account)
            .sasl(Sasl::plain(&account, PASSWORD))
            .request_caps(["sasl", "draft/persistence", "labeled-response"])
            .autojoin([&channel])
            .reconnect(Reconnect::exponential().max_delay(Duration::from_secs(1)))
            .connect()
            .await
            .unwrap();
        wait_for(
            &mut events,
            |event| matches!(event, Event::ChannelSynced { channel: joined } if joined == &channel),
        )
        .await;
        let persistence = client
            .persistence()
            .set(PersistenceSetting::On)
            .await
            .unwrap();
        assert!(persistence.effective);

        disconnect.send(()).await.unwrap();
        wait_for(&mut events, |event| {
            matches!(
                event,
                Event::Disconnected {
                    will_retry: true,
                    ..
                }
            )
        })
        .await;
        wait_for(&mut events, |event| matches!(event, Event::Reconnected(_))).await;
        assert!(client.state(|state| state.channel(&channel).is_some()));
        client
            .quit(Some("persistent reconnect verified"))
            .await
            .unwrap();
    })
    .await;
}

#[cfg(not(target_family = "wasm"))]
async fn disconnecting_proxy(target: SocketAddr) -> (SocketAddr, mpsc::Sender<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let (disconnect, mut disconnects) = mpsc::channel(1);
    tokio::spawn(async move {
        loop {
            let (mut client, _) = listener.accept().await.unwrap();
            let mut server = TcpStream::connect(target).await.unwrap();
            tokio::select! {
                _ = tokio::io::copy_bidirectional(&mut client, &mut server) => {}
                _ = disconnects.recv() => {}
            }
        }
    });
    (address, disconnect)
}

async fn wait_registered(events: &mut papo::Events, nick: &str) {
    loop {
        match events.next().await {
            Some(Event::Registered(welcome)) if welcome.nick == nick => return,
            Some(Event::Disconnected { reason, .. }) => panic!("disconnected: {reason}"),
            Some(_) => {}
            None => panic!("driver stopped before registration"),
        }
    }
}

#[cfg(not(target_family = "wasm"))]
async fn wait_for(events: &mut papo::Events, accepted: impl Fn(&Event) -> bool) {
    loop {
        let event = events.next().await.expect("driver stopped unexpectedly");
        if accepted(&event) {
            return;
        }
    }
}
