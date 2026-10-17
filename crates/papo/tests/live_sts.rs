//! STS upgrade behavior against a process-owned Ergo server.
#![cfg(all(not(target_family = "wasm"), tls))]

mod support;

use support::within;

use papo::{Client, Endpoint, Event};
use papo_test_server::{TestServer, unique};

#[tokio::test(flavor = "current_thread")]
async fn connect_waits_for_registration_after_an_sts_upgrade() {
    within(async {
        let server = TestServer::start_with_sts().unwrap();
        let nick = unique("papo-sts");
        let (client, mut events) = Client::builder(Endpoint::plaintext(
            "localhost",
            server.plaintext_addr().port(),
        ))
        .nick(&nick)
        .request_caps(["sts"])
        .tls_ca_pem(std::fs::read(server.ca_pem_path()).unwrap())
        .connect()
        .await
        .unwrap();

        assert!(client.state().is_connected());
        while let Some(event) = events.next().await {
            if matches!(event, Event::Registered(ref welcome) if welcome.nick == nick.as_str()) {
                client.quit(Some("STS upgrade complete")).await.unwrap();
                return;
            }
        }
        panic!("driver stopped before the registered event");
    })
    .await;
}
