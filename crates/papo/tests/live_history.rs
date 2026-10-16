//! History and read markers against the runner-managed Ergo server.
#![cfg(sockets)]

mod support;

use support::within;

use papo::{Client, Endpoint, Event, MessageEvent, Origin, cap};
use papo_test_server::{from_env, unique};

#[tokio::test(flavor = "current_thread")]
async fn pages_messages_by_id_and_round_trips_a_read_marker() {
    within(async {
        let server = from_env();
        let channel = format!("#{}", unique("papo-history"));
        let nick = unique("papo-history");
        let (client, mut events) = Client::builder(Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        ))
        .nick(&nick)
        .request_caps([
            "message-tags",
            "server-time",
            "batch",
            "echo-message",
            "labeled-response",
            "draft/chathistory",
            "draft/read-marker",
        ])
        .without_caps([cap::EVENT_PLAYBACK, cap::MULTILINE])
        .unwrap()
        .autojoin([&channel])
        .connect()
        .await
        .unwrap();
        wait_synced(&mut events, &channel).await;

        let texts = [
            "history begins with a deploy",
            "history records the migration",
            "history ends with a health check",
        ];
        let mut sent = Vec::new();
        for text in texts {
            sent.push(client.privmsg(&channel, text).await.unwrap());
        }

        let latest = client.history(&channel).latest(10).await.unwrap();
        let messages = chats(&latest.messages, &texts);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.text())
                .collect::<Vec<_>>(),
            texts
        );
        assert_eq!(
            messages
                .iter()
                .map(|message| message.msgid())
                .collect::<Vec<_>>(),
            sent.iter()
                .map(|message| message.msgid.as_deref())
                .collect::<Vec<_>>()
        );
        assert!(messages.iter().all(|message| {
            message.origin() == Origin::History && !message.is_live() && message.msgid().is_some()
        }));

        let third = sent[2].msgid.as_ref().expect("Ergo supplies a msgid");
        let before = client
            .history(&channel)
            .before(third)
            .limit(10)
            .await
            .unwrap();
        assert_eq!(
            chats(&before.messages, &texts)
                .iter()
                .map(|message| message.text())
                .collect::<Vec<_>>(),
            &texts[..2]
        );

        let newest = sent[2].time.expect("Ergo supplies server time");
        client.mark_read(&channel, newest).await.unwrap();
        assert_eq!(client.read_marker(&channel).await.unwrap(), Some(newest));
        client.quit(Some("history test complete")).await.unwrap();
    })
    .await;
}

fn chats<'a>(messages: &'a [MessageEvent], texts: &[&str]) -> Vec<&'a MessageEvent> {
    messages
        .iter()
        .filter(|message| texts.contains(&message.text()))
        .collect()
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
