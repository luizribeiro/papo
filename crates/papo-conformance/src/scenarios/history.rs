use std::time::{Duration, SystemTime};

use libtest_mimic::Failed;
use papo::{Error, HistoryItem, Origin, Sent, Source};
use papo_conformance_macros::scenario;
use papo_testing::Peer;

use super::support::join;
use crate::{Scenario, ScenarioResult};

fn read_marker_time() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000)
}

async fn history_peers(
    s: &Scenario,
    texts: &[&str],
) -> papo_testing::Result<(Peer, Peer, String, Vec<Sent>)> {
    let writer = s.connect("writer").await?;
    let channel = s.channel_name();
    writer.join(&channel).await?;
    let mut sent = Vec::with_capacity(texts.len());
    for text in texts {
        sent.push(writer.privmsg(&channel, *text).await?);
    }
    let reader = s.connect("reader").await?;
    reader.join(&channel).await?;
    Ok((writer, reader, channel, sent))
}

/// retrieves the latest channel history with replay metadata
#[scenario(
    requires(caps = ["batch", "draft/chathistory"]),
    covers = ["draft.chathistory.latest", "draft.chathistory.end-tag"]
)]
async fn draft_chathistory(s: Scenario) -> papo_testing::Result<()> {
    let texts = [
        "history begins with a deploy",
        "history records the migration",
        "history ends with a health check",
    ];
    let (writer, reader, channel, sent) = history_peers(&s, &texts).await?;
    let page = reader.history(&channel).latest(10).await?;
    assert_eq!(page.target.as_ref(), channel);
    assert!(page.end);
    let messages = page
        .messages()
        .filter(|message| texts.contains(&message.text()))
        .collect::<Vec<_>>();
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
            .map(|message| message.id())
            .collect::<Vec<_>>(),
        sent.iter()
            .map(|message| message.id.as_ref())
            .collect::<Vec<_>>()
    );
    for message in messages {
        assert_eq!(message.origin(), Origin::History);
        assert!(!message.is_live());
        assert_eq!(
            message.source().as_ref().and_then(Source::nick),
            Some(writer.nick().as_ref())
        );
        assert!(message.server_time.is_some());
    }
    Ok(())
}

fn message_texts(page: &papo::HistoryPage, expected: &[&str]) -> Vec<String> {
    page.messages()
        .filter(|message| expected.contains(&message.text()))
        .map(|message| message.text().to_owned())
        .collect()
}

/// pages channel history around message identifiers
#[scenario(
    requires(caps = ["batch", "draft/chathistory"]),
    covers = [
        "draft.chathistory.before-after-around",
        "draft.chathistory.between"
    ]
)]
async fn chathistory_paging(s: Scenario) -> papo_testing::Result<()> {
    let texts = [
        "release window opened",
        "database migration started",
        "database migration completed",
        "health checks passed",
        "release window closed",
    ];
    let (_, reader, channel, sent) = history_peers(&s, &texts).await?;
    let ids = sent
        .iter()
        .map(|message| message.id.as_ref().expect("Ergo supplies a message ID"))
        .collect::<Vec<_>>();

    let before = reader
        .history(&channel)
        .before(ids[3].as_ref())
        .limit(10)
        .await?;
    assert_eq!(message_texts(&before, &texts), texts[..3]);

    let after = reader
        .history(&channel)
        .after(ids[1].as_ref())
        .limit(10)
        .await?;
    assert_eq!(message_texts(&after, &texts), texts[2..]);

    let around = reader
        .history(&channel)
        .around(ids[2].as_ref())
        .limit(3)
        .await?;
    assert_eq!(message_texts(&around, &texts), texts[1..4]);

    let between = reader
        .history(&channel)
        .between(ids[0].as_ref(), ids[4].as_ref())
        .limit(10)
        .await?;
    assert_eq!(message_texts(&between, &texts), texts[1..4]);
    Ok(())
}

/// rejects history for an unavailable target with a typed server error
#[scenario(
    requires(caps = ["batch", "draft/chathistory"]),
    covers = ["draft.chathistory.errors"]
)]
async fn chathistory_errors(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let target = s.channel_name();
    let error = alice.history(&target).latest(10).await.unwrap_err();
    assert!(matches!(
        error,
        Error::Server(ref error) if error.code() == "INVALID_TARGET"
    ));
    Ok(())
}

/// includes typed channel activity when event playback is negotiated
#[scenario(
    requires(caps = ["batch", "draft/chathistory", "draft/event-playback"]),
    covers = ["draft.event-playback"]
)]
async fn event_playback(s: Scenario) -> papo_testing::Result<()> {
    let writer = s.connect("writer").await?;
    let channel = s.channel_name();
    writer.join(&channel).await?;
    writer
        .set_topic(&channel, "incident response completed")
        .await?;
    writer.privmsg(&channel, "service is healthy").await?;

    let reader = s.connect("reader").await?;
    reader.join(&channel).await?;
    let page = reader.history(&channel).latest(10).await?;
    let topic = page
        .items
        .iter()
        .find_map(|item| match item {
            HistoryItem::TopicChanged(topic) => Some(topic),
            _ => None,
        })
        .expect("history contains the topic change");
    assert_eq!(topic.channel.as_ref(), channel);
    assert_eq!(topic.topic.as_deref(), Some("incident response completed"));
    assert_eq!(topic.by.nick(), Some(writer.nick().as_ref()));
    Ok(())
}

/// sets and receives a channel read marker
#[scenario(requires(caps = ["draft/read-marker"]))]
async fn draft_read_marker(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let channel = format!("#{}", context.unique());
    let mut connection = context.client_caps(&nick, &["draft/read-marker"]).await?;
    join(&context, &mut connection, &channel).await?;
    let time = read_marker_time();
    let observed = connection
        .client
        .mark_read(&channel, time)
        .await
        .map_err(Failed::from)?;
    if observed != Some(time) {
        return Err("MARKREAD did not return the set timestamp".into());
    }
    connection.close().await
}

#[cfg(test)]
mod tests {
    #[test]
    fn read_marker_fixture_has_the_expected_unix_time() {
        assert_eq!(
            super::read_marker_time()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap(),
            std::time::Duration::new(1_760_529_342, 123_000_000)
        );
    }
}
