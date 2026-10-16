//! STS upgrade behavior against a process-owned Ergo server.
#![cfg(all(not(target_family = "wasm"), tls))]

use std::time::Duration;

use papo::{Client, Event};
use papo_test_server::{TestServer, unique};
use tokio::time::timeout;

#[tokio::test(flavor = "current_thread")]
async fn connect_waits_for_registration_after_an_sts_upgrade() {
    timeout(Duration::from_secs(10), async {
        let server = TestServer::start_with_sts().unwrap();
        let nick = unique("papo-sts");
        let (client, mut events) = Client::builder(&format!(
            "irc://localhost:{}",
            server.plaintext_addr().port()
        ))
        .nick(&nick)
        .caps(["sts"])
        .tls_ca_file(server.ca_pem_path())
        .unwrap()
        .connect()
        .await
        .unwrap();

        assert!(client.is_connected());
        while let Some(event) = events.next().await {
            if matches!(event, Event::Registered(ref welcome) if welcome.nick == nick) {
                client.quit(Some("STS upgrade complete")).await.unwrap();
                return;
            }
        }
        panic!("driver stopped before the registered event");
    })
    .await
    .expect("live STS upgrade did not finish within 10 seconds");
}
