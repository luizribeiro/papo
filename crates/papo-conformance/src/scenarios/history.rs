use libtest_mimic::Failed;
use papo::Event;
use papo_conformance_macros::scenario;

use super::support::{EXPECT_TIMEOUT, history_page, join};
use crate::{Scenario, ScenarioResult};

/// retrieves a channel history page
#[scenario(requires(caps = ["batch", "draft/chathistory"]))]
async fn draft_chathistory(context: Scenario) -> ScenarioResult {
    history_page(context).await
}

/// sets and receives a channel read marker
#[scenario(requires(caps = ["draft/read-marker"]))]
async fn draft_read_marker(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let channel = format!("#{}", context.unique());
    let mut connection = context.client_caps(&nick, &["draft/read-marker"]).await?;
    join(&context, &mut connection, &channel).await?;
    connection
        .client
        .send_raw(format!(
            "MARKREAD {channel} timestamp=2025-10-15T11:55:42.123Z"
        ))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| matches!(event, Event::ReadMarker(marker) if marker.target == channel.as_str() && marker.time.is_some()),
            EXPECT_TIMEOUT,
        )
        .await?;
    connection.close().await
}
