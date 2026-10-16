//! Catchup output against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

use std::{process::Command, time::Duration};

use papo::{Client, Event, Sasl};
use papo_test_server::{from_env, register_account, unique};
use tokio::time::timeout;

const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

#[tokio::test(flavor = "current_thread")]
async fn prints_three_unread_messages_and_advances_the_marker() {
    timeout(Duration::from_secs(20), async {
        let server = from_env();
        let address = format!("irc://{}", server.plaintext_addr());
        let channel = format!("#{}", unique("papo-catchup"));
        let account = unique("papo-catchup");
        let poster_nick = unique("papo-poster");
        register_account(&server, &account, PASSWORD).unwrap();

        let (poster, mut poster_events) = Client::builder(&address)
            .nick(&poster_nick)
            .channels([&channel])
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
            newest = poster.privmsg(&channel, text).await.unwrap().time;
        }

        let password = std::env::temp_dir().join(format!("{account}.password"));
        std::fs::write(&password, format!("{PASSWORD}\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_catchup"))
            .env("IRC_SERVER", &address)
            .env("IRC_NICK", &account)
            .env("IRC_PASSWORD_FILE", &password)
            .env("IRC_CHANNELS", &channel)
            .output()
            .unwrap();
        std::fs::remove_file(password).unwrap();
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

        let (reader, mut reader_events) = Client::builder(&address)
            .nick(&account)
            .sasl(Sasl::plain(&account, PASSWORD))
            .require_sasl(true)
            .caps(["draft/read-marker"])
            .channels([&channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut reader_events, &channel).await;
        assert!(reader.read_marker(&channel).await.unwrap() >= newest);
        reader.quit(Some("marker checked")).await.unwrap();
        poster.quit(Some("catchup test complete")).await.unwrap();
    })
    .await
    .expect("live catchup test did not finish within 20 seconds");
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
