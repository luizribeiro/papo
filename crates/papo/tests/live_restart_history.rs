//! History retention across a process-owned server restart.
#![cfg(not(target_family = "wasm"))]

mod support;

use papo::{Client, Endpoint, Origin, Sasl};
use papo_test_server::{TestServer, register_account_at, unique};
use support::within;

#[tokio::test(flavor = "current_thread")]
async fn restart_loses_memory_history() {
    within(async { assert!(!retains_message(false).await) }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn restart_retains_persistent_history() {
    within(async { assert!(retains_message(true).await) }).await;
}

async fn retains_message(persistent: bool) -> bool {
    let mut server = if persistent {
        TestServer::start_persistent().unwrap()
    } else {
        TestServer::start_default().unwrap()
    };
    let account = unique("papo-restart");
    let channel = format!("#{}", unique("papo-restart"));
    let password = "q7Hx-2mZp_9vLcT4wRfB";
    let text = if persistent {
        "persistent history survives the server restart"
    } else {
        "memory history ends with the server process"
    };
    register_account_at(server.plaintext_addr(), &account, password).unwrap();
    let (client, _events) = connect(&server, &account, password).await;
    client.join(support::channel(&channel)).await.unwrap();
    client.privmsg(&channel, text).await.unwrap();
    client.quit("restart fixture prepared").await.unwrap();

    server.restart().unwrap();
    let (client, _events) = connect(&server, &account, password).await;
    client.join(support::channel(&channel)).await.unwrap();
    let page = client.history(&channel).latest(10).await.unwrap();
    let retained = page
        .messages()
        .any(|message| message.text == text && message.origin == Origin::History);
    client.quit("restart history checked").await.unwrap();
    retained
}

async fn connect(server: &TestServer, account: &str, password: &str) -> (Client, papo::Events) {
    Client::builder(Endpoint::plaintext(
        "localhost",
        server.plaintext_addr().port(),
    ))
    .nick(support::nick(account))
    .sasl(Sasl::plain(account, password))
    .request_caps([
        "message-tags",
        "server-time",
        "batch",
        "echo-message",
        "labeled-response",
        "draft/chathistory",
    ])
    .connect()
    .await
    .unwrap()
}
