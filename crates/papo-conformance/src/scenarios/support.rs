use libtest_mimic::Failed;
use papo::Event;
use papo_proto::Message;

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
                matches!(event, Event::Joined { channel: joined, nick } if joined == &channel && nick == &second_nick)
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
        .send(Message::join(channel))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| matches!(event, Event::ChannelSynced { channel: joined } if joined == channel),
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

pub(super) async fn history_page(context: Scenario) -> ScenarioResult {
    let writer_nick = context.unique();
    let reader_nick = context.unique();
    let channel = format!("#{}", context.unique());
    let text = format!("history from {writer_nick}");
    let mut writer = context.client(&writer_nick).await?;
    join(&context, &mut writer, &channel).await?;
    writer
        .client
        .privmsg(&channel, &text)
        .await
        .map_err(Failed::from)?;
    let mut reader = context
        .client_caps(&reader_nick, &["draft/chathistory"])
        .await?;
    join(&context, &mut reader, &channel).await?;
    let response = reader
        .client
        .raw(Message::new("CHATHISTORY", ["LATEST", &channel, "*", "10"]))
        .await
        .map_err(Failed::from)?;
    let Some(history) = response.history else {
        return Err("CHATHISTORY response did not contain a history batch".into());
    };
    if !history
        .messages()
        .any(|message| message.as_ref().param(1) == Some(text.as_str()))
    {
        return Err("history batch did not contain the earlier message".into());
    }
    close_pair(writer, reader).await
}
