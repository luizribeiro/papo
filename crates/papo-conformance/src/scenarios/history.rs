use std::time::{Duration, SystemTime};

use libtest_mimic::Failed;
use papo::{Origin, Sent, Source};
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
