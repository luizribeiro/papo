//! Catchup output against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

use std::{process::Command, time::Duration};

use papo::{Client, Endpoint, Event, Line, Sasl};
use papo_test_server::{create_account, from_env, unique};
use tokio::time::timeout;

const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

#[tokio::test(flavor = "current_thread")]
async fn prints_three_unread_messages_and_advances_the_marker() {
    timeout(Duration::from_secs(20), async {
        let server = from_env();
        let endpoint = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        )
        .unwrap();
        let channel = format!("#{}", unique("papo-catchup"));
        let account = unique("papo-catchup");
        let poster_nick = unique("papo-poster");
        create_account(&server, &account, PASSWORD).unwrap();

        let (persistent, mut persistent_events) = Client::builder(endpoint.clone())
            .nick(&account)
            .allow_insecure_auth(true)
            .sasl(Sasl::plain(&account, PASSWORD))
            .request_caps(["sasl", "draft/persistence", "labeled-response"])
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut persistent_events, &channel).await;
        persistent
            .send_raw(Line::new("PERSISTENCE").with_params(["SET", "ON"]))
            .await
            .unwrap();
        persistent
            .quit("persistent catchup test account ready")
            .await
            .unwrap();

        let (poster, mut poster_events) = Client::builder(endpoint.clone())
            .nick(&poster_nick)
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut poster_events, &channel).await;
        let texts = [
            "the deployment started",
            "the migration completed",
            "the service is healthy",
        ];
        let mut newest = None;
        for text in texts {
            newest = poster.privmsg(&channel, text).await.unwrap().server_time;
        }

        let output = Command::new(env!("CARGO_BIN_EXE_catchup"))
            .arg(format!("irc://{}", server.plaintext_addr()))
            .env("IRC_NICK", &account)
            .env("IRC_PASSWORD", PASSWORD)
            .env("IRC_ALLOW_INSECURE_AUTH", "1")
            .env("IRC_CHANNEL", &channel)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            texts.map(|text| format!("{channel} {text}"))
        );

        let (reader, mut reader_events) = Client::builder(endpoint)
            .nick(&account)
            .allow_insecure_auth(true)
            .sasl(Sasl::plain(&account, PASSWORD))
            .request_caps(["draft/read-marker"])
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut reader_events, &channel).await;
        assert!(reader.fetch_read_marker(&channel).await.unwrap() >= newest);
        reader.quit("marker checked").await.unwrap();
        poster.quit("catchup test complete").await.unwrap();
    })
    .await
    .expect("live catchup test did not finish within 20 seconds");
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
