//! Ergo metadata and extension APIs against the runner-managed server.
#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use std::time::Duration;

use papo::{Client, Event};
#[cfg(not(target_family = "wasm"))]
use papo_test_server::TestServer;
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

#[tokio::test(flavor = "current_thread")]
#[cfg(not(target_family = "wasm"))]
async fn member_sees_channel_rename_and_relay_attribution() {
    timeout(Duration::from_secs(10), async {
        let server = TestServer::start_with_ephemeral_channels().unwrap();
        let address = format!("irc://{}", server.plaintext_addr());
        let old_channel = format!("#{}", unique("papo-rename"));
        let new_channel = format!("#{}", unique("papo-renamed"));
        let operator_nick = unique("papo-relay-operator");
        let member_nick = unique("papo-relay-member");
        let capabilities = [
            "draft/channel-rename",
            "draft/relaymsg",
            "labeled-response",
            "message-tags",
        ];
        let (operator, mut operator_events) = Client::builder(&address)
            .nick(&operator_nick)
            .caps(capabilities)
            .channels([&old_channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut operator_events, &old_channel).await;
        let (member, mut member_events) = Client::builder(&address)
            .nick(&member_nick)
            .caps(capabilities)
            .channels([&old_channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut member_events, &old_channel).await;
        operator
            .rename(&old_channel, &new_channel, "Project renamed")
            .await
            .unwrap();
        loop {
            match member_events.next().await {
                Some(Event::ChannelRenamed { old, new, reason }) if old == old_channel => {
                    assert_eq!(
                        (new, reason),
                        (new_channel.clone(), "Project renamed".into())
                    );
                    break;
                }
                Some(Event::Disconnected { reason, .. }) => panic!("disconnected: {reason}"),
                Some(_) => {}
                None => panic!("member stopped before the rename arrived"),
            }
        }

        operator
            .relay(&new_channel, "matrix/alice", "Hello from Matrix")
            .await
            .unwrap();
        loop {
            match member_events.next().await {
                Some(Event::Message(message)) if message.text() == "Hello from Matrix" => {
                    assert_eq!(message.relayed_by(), Some(operator_nick.as_str()));
                    break;
                }
                Some(Event::Disconnected { reason, .. }) => panic!("disconnected: {reason}"),
                Some(_) => {}
                None => panic!("member stopped before the relayed message arrived"),
            }
        }
        member.quit(Some("relay received")).await.unwrap();
        operator.quit(Some("relay sent")).await.unwrap();
    })
    .await
    .expect("live rename and relay test did not finish within 10 seconds");
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
