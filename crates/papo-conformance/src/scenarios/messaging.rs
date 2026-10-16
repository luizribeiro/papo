use libtest_mimic::Failed;
use papo::Event;
use papo_conformance_macros::scenario;
use papo_proto::{Message, Source, Typing};

use super::support::{EXPECT_TIMEOUT, close_pair, join, joined_pair};
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
                matches!(event, Event::TypingChanged(typing) if typing.target == channel && typing.state == Typing::Active)
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    close_pair(receiver, sender).await
}

/// reassembles a client multiline message
#[scenario(requires(caps = ["draft/multiline"]))]
async fn draft_multiline(context: Scenario) -> ScenarioResult {
    let (receiver, mut sender, channel) = joined_pair(&context).await?;
    let reference = context.unique().replace('_', "-");
    sender
        .client
        .send(Message::new(
            "BATCH",
            [
                format!("+{reference}"),
                "draft/multiline".into(),
                channel.clone(),
            ],
        ))
        .await
        .map_err(Failed::from)?;
    sender
        .client
        .send(Message::privmsg(&channel, "first ").with_tag("batch", reference.clone()))
        .await
        .map_err(Failed::from)?;
    sender
        .client
        .send(
            Message::privmsg(&channel, "second")
                .with_tag("batch", reference.clone())
                .with_tag("draft/multiline-concat", ""),
        )
        .await
        .map_err(Failed::from)?;
    sender
        .client
        .send(Message::new("BATCH", [format!("-{reference}")]))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut sender.events,
            |event| matches!(event, Event::Message(message) if message.raw_text() == "first second"),
            EXPECT_TIMEOUT,
        )
        .await?;
    close_pair(receiver, sender).await
}

/// delivers UTF-8 channel text without replacement
#[scenario(requires(isupport = ["UTF8ONLY"]))]
async fn utf8only(context: Scenario) -> ScenarioResult {
    let (mut receiver, sender, channel) = joined_pair(&context).await?;
    let text = "Olá, IRC 世界";
    sender
        .client
        .privmsg(&channel, text)
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut receiver.events,
            |event| matches!(event, Event::Message(message) if message.raw_text() == text),
            EXPECT_TIMEOUT,
        )
        .await?;
    close_pair(receiver, sender).await
}

/// redacts a delivered channel message
#[scenario(requires(caps = ["draft/message-redaction"]))]
async fn draft_message_redaction(context: Scenario) -> ScenarioResult {
    let sender_nick = context.unique();
    let receiver_nick = context.unique();
    let channel = format!("#{}", context.unique());
    let mut sender = context
        .client_caps(&sender_nick, &["draft/message-redaction"])
        .await?;
    join(&context, &mut sender, &channel).await?;
    let mut receiver = context
        .client_caps(&receiver_nick, &["draft/message-redaction"])
        .await?;
    join(&context, &mut receiver, &channel).await?;
    let response = sender
        .client
        .raw(Message::privmsg(&channel, "message to redact"))
        .await
        .map_err(Failed::from)?;
    let Some(sent) = response.sent() else {
        return Err("echoed message did not include a message identifier".into());
    };
    sender
        .client
        .send(Message::new(
            "REDACT",
            [&channel, &sent.msgid, "conformance redaction"],
        ))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut receiver.events,
            |event| matches!(event, Event::Redacted(redacted) if redacted.target == channel && redacted.msgid == sent.msgid),
            EXPECT_TIMEOUT,
        )
        .await?;
    close_pair(sender, receiver).await
}
