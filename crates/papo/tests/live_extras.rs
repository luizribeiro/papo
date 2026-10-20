//! Ergo metadata and extension APIs against the runner-managed server.
#![cfg(sockets)]

mod support;

use support::within;

use papo::{Client, Endpoint, Event};
#[cfg(all(not(target_family = "wasm"), feature = "unstable"))]
use papo_test_server::TestServer;
use papo_test_server::{from_env, unique};

#[tokio::test(flavor = "current_thread")]
async fn subscriber_receives_another_clients_metadata() {
    within(async {
        let server = from_env();
        let address = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        );
        let channel = format!("#{}", unique("papo-metadata"));
        let writer_nick = unique("papo-metadata-writer");
        let reader_nick = unique("papo-metadata-reader");
        let (writer, mut writer_events) = Client::builder(address.clone())
            .nick(support::nick(&writer_nick))
            .request_caps(["draft/metadata-2", "labeled-response"])
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut writer_events, &channel).await;
        let (reader, mut reader_events) = Client::builder(address)
            .nick(support::nick(&reader_nick))
            .request_caps(["draft/metadata-2", "labeled-response"])
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut reader_events, &channel).await;
        reader.metadata().subscribe(["status"]).await.unwrap();

        writer
            .metadata()
            .set("status", "available for review")
            .await
            .unwrap();
        loop {
            match reader_events.next().await {
                Some(Event::MetadataChanged(changed))
                    if matches!(&changed.subject, papo::Subject::User(nick) if nick == writer_nick.as_str())
                        && changed.key == "status" =>
                {
                    assert_eq!(changed.value.as_deref(), Some("available for review"));
                    break;
                }
                Some(Event::Disconnected(disconnected)) => {
                    panic!("disconnected: {disconnected}")
                }
                Some(_) => {}
                None => panic!("reader stopped before metadata arrived"),
            }
        }
        reader.quit("metadata received").await.unwrap();
        writer.quit("metadata sent").await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
#[cfg(all(not(target_family = "wasm"), feature = "unstable"))]
async fn member_sees_channel_rename_and_relay_attribution() {
    within(async {
        let server = TestServer::start_with_ephemeral_channels().unwrap();
        let address = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        );
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
        let (operator, mut operator_events) = Client::builder(address.clone())
            .nick(support::nick(&operator_nick))
            .request_caps(capabilities)
            .autojoin([&old_channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut operator_events, &old_channel).await;
        let (member, mut member_events) = Client::builder(address)
            .nick(support::nick(&member_nick))
            .request_caps(capabilities)
            .autojoin([&old_channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut member_events, &old_channel).await;
        operator
            .rename_channel(
                support::channel(&old_channel),
                support::channel(&new_channel),
                "Project renamed",
            )
            .await
            .unwrap();
        loop {
            match member_events.next().await {
                Some(Event::ChannelRenamed(renamed)) if renamed.old == old_channel.as_str() => {
                    assert_eq!(
                        (renamed.new, renamed.actor.nick(), renamed.reason.as_deref()),
                        (
                            support::channel(&new_channel),
                            Some(&support::nick(&operator_nick)),
                            Some("Project renamed")
                        )
                    );
                    break;
                }
                Some(Event::Disconnected(disconnected)) => {
                    panic!("disconnected: {disconnected}")
                }
                Some(_) => {}
                None => panic!("member stopped before the rename arrived"),
            }
        }

        operator
            .relay(
                support::channel(&new_channel),
                support::nick("matrix/alice"),
                "Hello from Matrix",
            )
            .await
            .unwrap();
        loop {
            match member_events.next().await {
                Some(Event::Message(message)) if message.text() == "Hello from Matrix" => {
                    assert_eq!(message.relayed_by(), Some(operator_nick.as_str()));
                    break;
                }
                Some(Event::Disconnected(disconnected)) => {
                    panic!("disconnected: {disconnected}")
                }
                Some(_) => {}
                None => panic!("member stopped before the relayed message arrived"),
            }
        }
        member.quit("relay received").await.unwrap();
        operator.quit("relay sent").await.unwrap();
    })
    .await;
}

async fn wait_synced(events: &mut papo::Events, channel: &str) {
    loop {
        match events.next().await {
            Some(Event::ChannelSynced(papo::event::ChannelSynced {
                channel: synced, ..
            })) if synced == channel => return,
            Some(Event::Disconnected(disconnected)) => {
                panic!("disconnected: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while joining"),
        }
    }
}
