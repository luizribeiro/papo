use libtest_mimic::Failed;
use papo::Event;
use papo_conformance_macros::scenario;
use papo_proto::{Message, Source};

use super::support::EXPECT_TIMEOUT;
use crate::{Scenario, ScenarioResult};

/// echoes a channel PRIVMSG to its sender
#[scenario(requires(caps = ["echo-message"]))]
async fn echo_message(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let channel = format!("#{}", context.unique());
    let text = format!("echo from {}", context.unique());
    let mut connection = context.client(&nick).await?;
    let registered_nick = connection.welcome.nick.clone();
    connection
        .client
        .send(Message::join(&channel))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| {
                matches!(
                    event,
                    Event::Joined {
                        channel: joined,
                        nick: joined_nick,
                    } if joined == &channel && joined_nick == &registered_nick
                )
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    connection
        .client
        .privmsg(&channel, &text)
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| {
                matches!(event, Event::Message(message) if {
                    let message = message.as_ref();
                    message.command_is("PRIVMSG")
                        && message.param(0) == Some(channel.as_str())
                        && message.param(1) == Some(text.as_str())
                        && matches!(message.source(), Some(Source::User { nick, .. }) if nick == registered_nick)
                })
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    connection.close().await
}
