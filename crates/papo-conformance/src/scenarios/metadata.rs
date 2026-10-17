use libtest_mimic::Failed;
use papo::Event;
use papo_conformance_macros::scenario;

use super::support::{EXPECT_TIMEOUT, join};
use crate::{Scenario, ScenarioResult};

/// sets and receives client metadata
#[scenario(requires(caps = ["batch", "draft/metadata-2"]))]
async fn draft_metadata_2(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let key = "example.org/papo-conformance";
    let value = context.unique();
    let mut connection = context.client(&nick).await?;
    connection
        .client
        .send_raw(format!("METADATA {nick} SET {key} {value}"))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| matches!(event, Event::MetadataChanged(changed) if changed.target == nick.as_str() && changed.key == key && changed.value.as_deref() == Some(value.as_str())),
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
    connection
        .client
        .send_raw(format!("RENAME {old} {new} :conformance rename"))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| matches!(event, Event::ChannelRenamed(renamed) if renamed.old == old.as_str() && renamed.new == new.as_str()),
            EXPECT_TIMEOUT,
        )
        .await?;
    connection.close().await
}
