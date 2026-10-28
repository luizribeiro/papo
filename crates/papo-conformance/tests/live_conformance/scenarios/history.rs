use std::time::{Duration, SystemTime};

use papo::{Error, Event, MessageId, Sent};
use papo_conformance_macros::scenario;

use crate::scenarios::support::{confirmed_line, conversation_name};
use crate::{Peer, Scenario};

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
    let page = reader.history(&channel).latest().limit(10).await?;
    assert_eq!(
        conversation_name(&page.conversation),
        Some(channel.as_str())
    );
    assert!(page.end);
    let messages = page
        .chat_messages()
        .filter(|message| texts.contains(&message.text.as_str()))
        .collect::<Vec<_>>();
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
            .map(|message| message.msgid.as_ref())
            .collect::<Vec<_>>(),
        sent.iter()
            .map(|message| Some(confirmed_line(message).0))
            .collect::<Vec<_>>()
    );
    for message in messages {
        assert_eq!(
            crate::source_nick(&message.actor),
            Some(writer.nick().as_ref())
        );
        assert!(message.tags.get("time").is_some());
    }
    Ok(())
}

/// replays channel history through Ergo's legacy command
#[scenario(requires(caps = ["batch"]), covers = ["ergo.history-legacy"])]
async fn legacy_history(s: Scenario) -> papo_testing::Result<()> {
    let profile = std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into());
    if !matches!(profile.as_str(), "ergo-default" | "ergo-persistent") {
        return s.skip("the selected profile does not implement Ergo HISTORY");
    }
    let text = "legacy history checkpoint";
    let (_, mut reader, channel, _) = history_peers(&s, &[text]).await?;
    reader
        .send_raw(papo::Line::new("HISTORY").with_params([channel.as_str(), "10"]))
        .await?;
    let page = reader.wait_for(Event::as_history).await?;
    assert_eq!(
        conversation_name(&page.conversation),
        Some(channel.as_str())
    );
    page.chat_messages()
        .find(|message| message.text == text)
        .expect("HISTORY replays the stored message");
    Ok(())
}

fn message_texts(page: &papo::HistoryPage, expected: &[&str]) -> Vec<String> {
    page.chat_messages()
        .filter(|message| expected.contains(&message.text.as_str()))
        .map(|message| message.text.clone())
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
        .map(|message| confirmed_line(message).0)
        .collect::<Vec<_>>();

    let before = reader.history(&channel).before(ids[3]).limit(10).await?;
    assert_eq!(message_texts(&before, &texts), texts[..3]);

    let after = reader.history(&channel).after(ids[1]).limit(10).await?;
    assert_eq!(message_texts(&after, &texts), texts[2..]);

    let around = reader.history(&channel).around(ids[2]).limit(3).await?;
    assert_eq!(message_texts(&around, &texts), texts[1..4]);

    let between = reader
        .history(&channel)
        .between(ids[0], ids[4])
        .limit(10)
        .await?;
    assert_eq!(message_texts(&between, &texts), texts[1..4]);
    Ok(())
}

/// pages backward until the server proves that no older messages remain
#[scenario(
    requires(caps = ["batch", "draft/chathistory"]),
    covers = ["draft.chathistory.end-pagination"]
)]
async fn terminal_history_page(s: Scenario) -> papo_testing::Result<()> {
    enum Cursor {
        Id(String),
        Time(SystemTime),
    }

    let alice = s.connect("alice").await?;
    let channel = alice
        .state()
        .name_rules()
        .expect("server advertised CASEMAPPING")
        .channel(s.channel_name());
    alice.join(&channel).await?;
    let texts = (1..=25)
        .map(|number| format!("deployment log entry {number:02}"))
        .collect::<Vec<_>>();
    for text in &texts {
        alice.privmsg(&channel, text).await?;
    }

    let mut cursor = None;
    let mut remaining = texts.len();
    loop {
        let page = match cursor {
            None => alice.history(&*channel).latest().limit(10).await?,
            Some(Cursor::Id(ref id)) => {
                alice
                    .history(&*channel)
                    .before(MessageId::from(id.clone()))
                    .limit(10)
                    .await?
            }
            Some(Cursor::Time(time)) => alice.history(&*channel).before(time).limit(10).await?,
        };
        let messages = page.chat_messages().collect::<Vec<_>>();
        let page_texts = messages
            .iter()
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>();
        let start = remaining.saturating_sub(10);
        assert_eq!(page_texts, texts[start..remaining]);
        if start == 0 {
            assert!(page.end, "the final history page must report the end");
        }
        if page.end {
            assert_eq!(start, 0, "history ended while older messages remained");
            break;
        }
        assert!(!messages.is_empty(), "an empty page must report the end");
        let oldest = messages.first().expect("nonempty page");
        cursor = Some(match oldest.msgid.as_ref() {
            Some(id) => Cursor::Id(id.as_ref().to_owned()),
            None => Cursor::Time(oldest.server_time.expect("history has server time")),
        });
        remaining = start;
    }
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
    let error = alice.history(&target).latest().limit(10).await.unwrap_err();
    assert!(matches!(
        error,
        Error::Server(ref error) if error.code() == "INVALID_TARGET"
    ));
    Ok(())
}

/// lists recently active history targets with their newest timestamps
#[scenario(
    requires(caps = ["batch", "draft/chathistory"]),
    covers = ["draft.chathistory.targets"]
)]
async fn chathistory_targets(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect_as_account("alice").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    let sent = alice.privmsg(&channel, "target history checkpoint").await?;
    let time = confirmed_line(&sent)
        .2
        .expect("server-time accompanies the labelled echo");

    let targets = alice
        .recent_conversations(
            time - Duration::from_secs(60),
            time + Duration::from_secs(60),
        )
        .await?;
    let target = targets
        .iter()
        .find(|target| {
            crate::scenarios::support::conversation_name(&target.conversation)
                == Some(channel.as_str())
        })
        .expect("the active channel is returned");
    assert_eq!(target.latest_server_time, time);
    Ok(())
}

/// soju excludes its server notices from history pages and target listings
#[scenario(
    requires(caps = ["batch", "draft/chathistory"]),
    covers = ["soju.server-notice-history"]
)]
async fn soju_server_notice_history(s: Scenario) -> papo_testing::Result<()> {
    if std::env::var("PAPO_TEST_PROFILE").as_deref() != Ok("soju-ergo") {
        return s.skip("the selected profile does not expose soju's BouncerServ");
    }
    let mut alice = s.connect("alice").await?;
    let text = papo_test_server::unique("history-notice");
    let bouncer = alice.direct("BouncerServ").await?;
    alice
        .privmsg(&bouncer, format!("server notice {text}"))
        .await?;
    let notice = alice
        .wait_for(|event| {
            event.as_server_notice().filter(|message| {
                message.kind == papo::event::ServerNoticeKind::Notice && message.text == text
            })
        })
        .await?;
    assert_eq!(crate::source_nick(&notice.actor), Some("BouncerServ"));
    let papo::event::Audience::Broadcast(papo::event::Broadcast::Mask(target)) = &notice.to else {
        panic!("server notice was not sent to a mask");
    };
    let target = target.to_string();
    assert!(target.starts_with('$'));

    let page = alice.history("BouncerServ").latest().limit(10).await?;
    assert!(page.items.is_empty());
    assert!(page.end);
    let targets = alice
        .recent_conversations(
            SystemTime::UNIX_EPOCH,
            SystemTime::now() + Duration::from_secs(60),
        )
        .await?;
    let bouncer = alice
        .state()
        .name_rules()
        .expect("server advertised CASEMAPPING")
        .nick("BouncerServ");
    assert!(targets.iter().all(|entry| {
        let Some(name) = crate::scenarios::support::conversation_name(&entry.conversation) else {
            return true;
        };
        name != target && bouncer != name
    }));
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
    let page = reader.history(&channel).latest().limit(10).await?;
    let topic = page
        .items
        .iter()
        .find_map(|item| match item {
            Event::TopicChanged(topic) => Some(topic),
            _ => None,
        })
        .expect("history contains the topic change");
    assert_eq!(topic.channel.as_ref(), channel);
    assert_eq!(topic.topic.as_deref(), Some("incident response completed"));
    assert_eq!(
        crate::source_nick(&topic.actor),
        Some(writer.nick().as_ref())
    );
    Ok(())
}

/// sets, fetches, and monotonically advances a channel read marker
#[scenario(
    requires(caps = ["draft/read-marker"]),
    covers = ["draft.read-marker.set-get", "draft.read-marker.monotonic"]
)]
async fn draft_read_marker(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    let time = read_marker_time();
    assert_eq!(alice.mark_read(&channel, time).await?, Some(time));
    assert_eq!(
        alice.state().read_marker(&channel),
        Some(papo::ReadMarker::ReadUntil(time))
    );
    assert_eq!(alice.fetch_read_marker(&channel).await?, Some(time));

    let older = time - Duration::from_secs(60);
    assert_eq!(alice.mark_read(&channel, older).await?, Some(time));
    assert_eq!(
        alice.state().read_marker(&channel),
        Some(papo::ReadMarker::ReadUntil(time))
    );
    assert_eq!(alice.fetch_read_marker(&channel).await?, Some(time));
    Ok(())
}

/// replays an account's stored read marker when another session joins
#[scenario(
    requires(caps = ["draft/read-marker"]),
    covers = ["draft.read-marker.on-join"]
)]
async fn read_marker_on_join(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, account) = s.connect_as_account_with_key("alice").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    let initial = alice.wait_for(Event::as_read_marker_changed).await?;
    assert_eq!(
        conversation_name(&initial.conversation),
        Some(channel.as_str())
    );
    assert_eq!(initial.read_until, None);
    assert_eq!(
        alice.state().read_marker(&channel),
        Some(papo::ReadMarker::Unset)
    );

    let time = read_marker_time();
    alice.mark_read(&channel, time).await?;
    let stored = alice.wait_for(Event::as_read_marker_changed).await?;
    assert_eq!(
        conversation_name(&stored.conversation),
        Some(channel.as_str())
    );
    assert_eq!(stored.read_until, Some(time));

    let mut phone = s.connect_account_session("alice-phone", account).await?;
    let mut early_marker = None;
    let synced = phone
        .wait_for(|event| {
            if let Some(marker) = event
                .as_read_marker_changed()
                .filter(|marker| conversation_name(&marker.conversation) == Some(channel.as_str()))
            {
                early_marker = Some(marker.clone());
            }
            event
                .as_channel_synced()
                .filter(|synced| synced.channel.as_ref() == channel)
        })
        .await?;
    assert_eq!(synced.channel.as_ref(), channel);
    assert!(phone.state().channel(&channel).is_some());
    let replayed = match early_marker {
        Some(marker) => marker,
        None => phone.wait_for(Event::as_read_marker_changed).await?,
    };
    assert_eq!(
        conversation_name(&replayed.conversation),
        Some(channel.as_str())
    );
    assert_eq!(replayed.read_until, Some(time));
    assert_eq!(
        phone.state().read_marker(&channel),
        Some(papo::ReadMarker::ReadUntil(time))
    );
    Ok(())
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
