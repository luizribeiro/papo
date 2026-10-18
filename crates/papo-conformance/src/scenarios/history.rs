use std::time::{Duration, SystemTime};

use libtest_mimic::Failed;
use papo_conformance_macros::scenario;

use super::support::{history_page, join};
use crate::{Scenario, ScenarioResult};

fn read_marker_time() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000)
}

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
