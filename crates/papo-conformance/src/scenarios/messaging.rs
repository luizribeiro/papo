use libtest_mimic::Failed;
use papo::Event;
use papo_conformance_macros::scenario;
use papo_proto::{Message, Source, Typing};

use super::support::{EXPECT_TIMEOUT, close_pair, joined_pair};
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

/// delivers a channel message with server time
#[scenario(requires(caps = ["server-time"]))]
async fn server_time(context: Scenario) -> ScenarioResult {
    let (mut receiver, sender, channel) = joined_pair(&context).await?;
    let text = format!("timestamped message from {}", sender.welcome.nick);
    sender
        .client
        .privmsg(&channel, &text)
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut receiver.events,
            |event| {
                matches!(event, Event::Message(message) if message.raw_text() == text && message.as_ref().tag("time").is_some())
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    close_pair(receiver, sender).await
}

/// delivers a client-only typing tag
#[scenario(requires(caps = ["message-tags"]))]
async fn message_tags(context: Scenario) -> ScenarioResult {
    let (mut receiver, sender, channel) = joined_pair(&context).await?;
    sender
        .client
        .send(Message::new("TAGMSG", [&channel]).with_tag("+typing", "active"))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut receiver.events,
            |event| {
                matches!(event, Event::Typing(message) if message.target() == channel && message.typing() == Some(Typing::Active))
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    close_pair(receiver, sender).await
}
