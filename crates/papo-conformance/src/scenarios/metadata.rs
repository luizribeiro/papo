use libtest_mimic::Failed;
use papo::Event;
use papo_conformance_macros::scenario;
use papo_proto::Message;

use super::support::{EXPECT_TIMEOUT, join};
use crate::{Scenario, ScenarioResult};

/// sets and receives client metadata
#[scenario(requires(caps = ["batch", "draft/metadata-2"]))]
async fn draft_metadata_2(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let key = "example.org/papo-conformance";
    let value = context.unique();
    let mut connection = context.client(&nick).await?;
    let response = connection
        .client
        .raw(Message::new("METADATA", [&nick, "SET", key, &value]))
        .await
        .map_err(Failed::from)?;
    if let Some(error) = response.error() {
        return Err(format!("server rejected METADATA SET: {error:?}").into());
    }
    context
        .expect(
            &mut connection.events,
            |event| matches!(event, Event::MetadataChanged { target, key: changed_key, value: Some(changed_value), .. } if target == &nick && changed_key == key && changed_value == &value),
            EXPECT_TIMEOUT,
        )
        .await?;
    connection.close().await
}

/// renames a joined channel
#[scenario(requires(caps = ["draft/channel-rename"]))]
async fn draft_channel_rename(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let old = format!("#{}", context.unique());
    let new = format!("#{}", context.unique());
    let mut connection = context
        .client_caps(&nick, &["draft/channel-rename"])
        .await?;
    join(&context, &mut connection, &old).await?;
    let response = connection
        .client
        .raw(Message::new("RENAME", [&old, &new, "conformance rename"]))
        .await
        .map_err(Failed::from)?;
    if let Some(error) = response.error() {
        return Err(format!("server rejected RENAME: {error:?}").into());
    }
    context
        .expect(
            &mut connection.events,
            |event| matches!(event, Event::ChannelRenamed { old: renamed, new: replacement, .. } if renamed == &old && replacement == &new),
            EXPECT_TIMEOUT,
        )
        .await?;
    connection.close().await
}
