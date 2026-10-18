#![allow(dead_code)]

use libtest_mimic::Failed;
use papo::Event;

use crate::{Scenario, ScenarioResult};

pub(super) const EXPECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
pub(super) const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

pub(super) async fn joined_pair(
    context: &Scenario,
) -> Result<(crate::RegisteredClient, crate::RegisteredClient, String), Failed> {
    let first_nick = context.unique();
    let second_nick = context.unique();
    let channel = format!("#{}", context.unique());
    let mut first = context.client(&first_nick).await?;
    join(context, &mut first, &channel).await?;
    let mut second = context.client(&second_nick).await?;
    join(context, &mut second, &channel).await?;
    context
        .expect(
            &mut first.events,
            |event| {
                matches!(event, Event::Joined(papo::event::Joined { channel: joined, nick, .. }) if joined == channel.as_str() && nick == second_nick.as_str())
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    Ok((first, second, channel))
}

pub(super) async fn join(
    context: &Scenario,
    connection: &mut crate::RegisteredClient,
    channel: &str,
) -> Result<(), Failed> {
    connection
        .client
        .join(channel)
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| matches!(event, Event::ChannelSynced(papo::event::ChannelSynced { channel: joined, .. }) if joined == channel),
            EXPECT_TIMEOUT,
        )
        .await?;
    Ok(())
}

pub(super) async fn close_pair(
    first: crate::RegisteredClient,
    second: crate::RegisteredClient,
) -> ScenarioResult {
    first.close().await?;
    second.close().await
}
