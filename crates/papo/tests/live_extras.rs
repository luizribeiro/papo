//! Ergo metadata and extension APIs against the runner-managed server.
#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use std::time::Duration;

use papo::{Client, Event};
use papo_test_server::{from_env, unique};
use tokio::time::timeout;

#[tokio::test(flavor = "current_thread")]
async fn subscriber_receives_another_clients_metadata() {
    timeout(Duration::from_secs(10), async {
        let server = from_env();
        let address = format!("irc://{}", server.plaintext_addr());
        let channel = format!("#{}", unique("papo-metadata"));
        let writer_nick = unique("papo-metadata-writer");
        let reader_nick = unique("papo-metadata-reader");
        let (writer, mut writer_events) = Client::builder(&address)
            .nick(&writer_nick)
            .caps(["draft/metadata-2", "labeled-response"])
            .channels([&channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut writer_events, &channel).await;
        let (reader, mut reader_events) = Client::builder(&address)
            .nick(&reader_nick)
            .caps(["draft/metadata-2", "labeled-response"])
            .channels([&channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut reader_events, &channel).await;
        reader.metadata().subscribe(["status"]).await.unwrap();

        writer
            .metadata()
            .set("status", Some("available for review"))
            .await
            .unwrap();
        loop {
            match reader_events.next().await {
                Some(Event::MetadataChanged {
                    target,
                    key,
                    value: Some(value),
                    ..
                }) if target == writer_nick && key == "status" => {
                    assert_eq!(value, "available for review");
                    break;
                }
                Some(Event::Disconnected { reason, .. }) => panic!("disconnected: {reason}"),
                Some(_) => {}
                None => panic!("reader stopped before metadata arrived"),
            }
        }
        reader.quit(Some("metadata received")).await.unwrap();
        writer.quit(Some("metadata sent")).await.unwrap();
    })
    .await
    .expect("live metadata test did not finish within 10 seconds");
}

async fn wait_synced(events: &mut papo::Events, channel: &str) {
    loop {
        match events.next().await {
            Some(Event::ChannelSynced { channel: synced }) if synced == channel => return,
            Some(Event::Disconnected { reason, .. }) => panic!("disconnected: {reason}"),
            Some(_) => {}
            None => panic!("driver stopped while joining"),
        }
    }
}
