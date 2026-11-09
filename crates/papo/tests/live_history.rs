//! History and read markers against the runner-managed Ergo server.
#![cfg(sockets)]

mod support;

use support::within;

use papo::{Client, Endpoint, Event, Feature, Message as ChatMessage};
use papo_test_server::{OPER_NAME, OPER_PASSWORD, from_env, unique};

#[tokio::test(flavor = "current_thread")]
async fn pages_messages_by_id_and_round_trips_a_read_marker() {
    within(async {
        let server = from_env();
        let channel = format!("#{}", unique("papo-history"));
        let nick = unique("papo-history");
        let (client, mut events) = Client::builder(
            Endpoint::plaintext(
                server.plaintext_addr().ip().to_string(),
                server.plaintext_addr().port(),
            )
            .unwrap(),
            support::nick(&nick),
        )
        .disable(Feature::EventPlayback)
        .disable(Feature::Multiline)
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

        let latest = client.history(&channel).latest().limit(10).await.unwrap();
        let messages = chats(latest.chat_messages(), &texts);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            texts
        );
        assert_eq!(
            messages
                .iter()
                .map(|message| message.msgid().map(AsRef::as_ref))
                .collect::<Vec<_>>(),
            sent.iter()
                .map(|message| Some(confirmed_line(message).0.as_ref()))
                .collect::<Vec<_>>()
        );
        assert!(messages.iter().all(|message| message.msgid().is_some()));

        let (third, _, newest) = confirmed_line(&sent[2]);
        let before = client
            .history(&channel)
            .before(third)
            .limit(10)
            .await
            .unwrap();
        assert_eq!(
            chats(before.chat_messages(), &texts)
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            &texts[..2]
        );

        let newest = newest.expect("Ergo supplies server time");
        assert_eq!(
            client.mark_read(&channel).await.unwrap(),
            papo::MarkReadOutcome::Advanced(newest)
        );
        assert_eq!(
            client.fetch_read_marker(&channel).await.unwrap(),
            Some(newest)
        );
        client.quit("history test complete").await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn latest_after_returns_the_newest_page_and_reports_a_gap() {
    within(async {
        let server = from_env();
        let channel = format!("#{}", unique("papo-latest-after"));
        let nick = unique("papo-latest-after");
        let (client, mut events) = Client::builder(
            Endpoint::plaintext(
                server.plaintext_addr().ip().to_string(),
                server.plaintext_addr().port(),
            )
            .unwrap(),
            support::nick(&nick),
        )
        .disable(Feature::EventPlayback)
        .disable(Feature::Multiline)
        .autojoin([&channel])
        .connect()
        .await
        .unwrap();
        wait_synced(&mut events, &channel).await;

        let texts = [
            "history saved the rollout plan",
            "history saved the migration start",
            "history saved the schema update",
            "history saved the service restart",
            "history saved the final health check",
        ];
        let mut sent = Vec::new();
        for text in texts {
            sent.push(client.privmsg(&channel, text).await.unwrap());
        }
        let cursor = confirmed_line(&sent[1]).0.clone();

        let newest = client
            .history(&channel)
            .latest_after(cursor.clone())
            .limit(2)
            .await
            .unwrap();
        assert_eq!(
            chats(newest.chat_messages(), &texts)
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            &texts[3..]
        );
        assert!(!newest.end);

        let complete = client
            .history(&channel)
            .latest_after(cursor)
            .limit(10)
            .await
            .unwrap();
        assert_eq!(
            chats(complete.chat_messages(), &texts)
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            &texts[2..]
        );
        assert!(complete.end);
        client.quit("latest history test complete").await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn cursor_pages_every_message_once() {
    within(async {
        let server = from_env();
        let channel = format!("#{}", unique("papo-history-cursor"));
        let nick = unique("papo-history-cursor");
        let (client, mut events) = Client::builder(
            Endpoint::plaintext(
                server.plaintext_addr().ip().to_string(),
                server.plaintext_addr().port(),
            )
            .unwrap(),
            support::nick(&nick),
        )
        .disable(Feature::EventPlayback)
        .disable(Feature::Multiline)
        .rate_limit(papo::RateLimit::unlimited())
        .allow_insecure_auth(true)
        .autojoin([&channel])
        .connect()
        .await
        .unwrap();
        wait_synced(&mut events, &channel).await;
        client
            .oper(OPER_NAME, papo::Secret::new(OPER_PASSWORD))
            .await
            .unwrap();

        let expected = (1..=250)
            .map(|number| format!("history cursor entry {number:03}"))
            .collect::<Vec<_>>();
        let mut sends = tokio::task::JoinSet::new();
        for text in &expected {
            let client = client.clone();
            let line = papo::Line::privmsg(&channel, text);
            sends.spawn(async move { client.send_raw(line).await });
        }
        while let Some(sent) = sends.join_next().await {
            sent.unwrap().unwrap();
        }

        let mut pages = Vec::new();
        let mut page = client.history(&channel).latest().limit(100).await.unwrap();
        loop {
            pages.push(
                page.chat_messages()
                    .filter(|message| message.text.starts_with("history cursor entry "))
                    .map(|message| message.text.clone())
                    .collect::<Vec<_>>(),
            );
            if page.end {
                break;
            }
            page = client
                .history(&channel)
                .before(page.cursor().expect("a nonempty page has a cursor"))
                .limit(100)
                .await
                .unwrap();
        }
        let mut actual = pages.into_iter().flatten().collect::<Vec<_>>();
        actual.sort();
        assert_eq!(actual, expected);
        client.quit("cursor history test complete").await.unwrap();
    })
    .await;
}

fn confirmed_line(sent: &papo::Sent) -> (&papo::MessageId, &str, Option<papo::ServerTime>) {
    let papo::SentOutcome::Confirmed {
        lines, server_time, ..
    } = sent.outcome()
    else {
        panic!("Ergo should confirm history seed messages")
    };
    let [(id, text)] = lines.as_slice() else {
        panic!("each history seed should be sent as one line")
    };
    (id, text, *server_time)
}

fn chats<'a>(
    messages: impl Iterator<Item = &'a ChatMessage>,
    texts: &[&str],
) -> Vec<&'a ChatMessage> {
    messages
        .filter(|message| texts.contains(&message.text.as_str()))
        .collect()
}

async fn wait_synced(events: &mut papo::Events, channel: &str) {
    loop {
        match events.next().await.map(|(event, _)| event) {
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
