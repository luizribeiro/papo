use libtest_mimic::Failed;
use papo::{Event, MessageKind, Source};
use papo_conformance_macros::scenario;
use papo_proto::Typing;

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
        .join(&channel)
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| {
                matches!(
                    event,
                    Event::Joined(papo::event::Joined { channel: joined,
                        nick: joined_nick, .. }) if joined == channel.as_str() && joined_nick == registered_nick.as_str()
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
                    matches!(message.kind(), MessageKind::Privmsg)
                        && message.target() == channel
                        && message.raw_text() == text
                        && matches!(message.source(), Some(Source::User { nick, .. }) if nick == registered_nick.as_str())
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
                matches!(event, Event::Message(message) if message.raw_text() == text && message.tags().contains("time"))
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
        .send_raw(&format!("@+typing=active TAGMSG {channel}"))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut receiver.events,
            |event| {
                matches!(event, Event::TypingChanged(typing) if typing.target == channel.as_str() && typing.state == Typing::Active)
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
        .send_raw(&format!("BATCH +{reference} draft/multiline {channel}"))
        .await
        .map_err(Failed::from)?;
    sender
        .client
        .send_raw(&format!("@batch={reference} PRIVMSG {channel} :first "))
        .await
        .map_err(Failed::from)?;
    sender
        .client
        .send_raw(&format!(
            "@batch={reference};draft/multiline-concat PRIVMSG {channel} second"
        ))
        .await
        .map_err(Failed::from)?;
    sender
        .client
        .send_raw(&format!("BATCH -{reference}"))
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
    let sent = sender
        .client
        .privmsg(&channel, "message to redact")
        .await
        .map_err(Failed::from)?;
    let Some(id) = sent.id else {
        return Err("echoed message did not include a message identifier".into());
    };
    sender
        .client
        .redact(
            &papo::Target::from(&channel),
            &id,
            Some("conformance redaction"),
        )
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut receiver.events,
            |event| matches!(event, Event::Redacted(redacted) if redacted.target == channel.as_str() && redacted.message == id),
            EXPECT_TIMEOUT,
        )
        .await?;
    close_pair(sender, receiver).await
}
