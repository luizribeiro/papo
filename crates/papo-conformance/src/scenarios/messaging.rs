use libtest_mimic::{Completion, Failed};
use papo::{Event, MessageKind, Origin, Source};
use papo_conformance_macros::scenario;
use papo_proto::Typing;
use papo_testing::Peer;

use super::support::{EXPECT_TIMEOUT, close_pair, join, joined_pair};
use crate::{Scenario, ScenarioResult};

async fn fixture_pair(context: &Scenario) -> Result<(Peer, Peer, String), Failed> {
    let mut alice = context.connect("alice").await?;
    let bob = context.connect("bob").await?;
    let channel = context.channel_name();
    alice.join(&channel).await.map_err(Failed::from)?;
    bob.join(&channel).await.map_err(Failed::from)?;
    let bob_nick = bob.nick();
    let joined = alice
        .wait_for(|event| event.as_joined().filter(|joined| joined.nick == bob_nick))
        .await?;
    assert_eq!(joined.channel.as_ref(), channel);
    assert_eq!(joined.nick, bob_nick);
    assert!(!joined.is_self);
    for peer in [&alice, &bob] {
        let state = peer.state();
        let members = state.channel(&channel).expect("joined channel is in state");
        assert!(members.is_synced());
        assert!(members.member(alice.nick()).is_some());
        assert!(members.member(bob.nick()).is_some());
    }
    Ok((alice, bob, channel))
}

/// a channel PRIVMSG has one correlated echo and matching recipient metadata
#[scenario(
    requires(caps = ["echo-message", "labeled-response", "message-tags", "server-time"]),
    covers = ["base.privmsg.channel", "ircv3.echo-message"]
)]
async fn echo_message(context: Scenario) -> ScenarioResult {
    let (mut alice, mut bob, channel) = fixture_pair(&context).await?;
    let sent = alice.privmsg(&channel, "hello from alice").await?;
    let echo = alice.wait_for(Event::as_message).await?;
    let message = bob.wait_for(Event::as_message).await?;
    assert_eq!(echo.kind(), MessageKind::Privmsg);
    assert_eq!(echo.target(), channel);
    assert_eq!(echo.raw_text(), "hello from alice");
    assert_eq!(echo.id(), sent.id.as_ref());
    assert_eq!(echo.server_time, sent.time);
    assert_eq!(echo.origin(), Origin::Echo);
    assert_eq!(message.kind(), MessageKind::Privmsg);
    assert_eq!(
        message.source().as_ref().and_then(Source::nick),
        Some(alice.nick().as_ref())
    );
    assert_eq!(message.target(), channel);
    assert_eq!(message.raw_text(), "hello from alice");
    assert_eq!(message.text(), "hello from alice");
    assert_eq!(message.id(), sent.id.as_ref());
    assert_eq!(message.server_time, sent.time);
    assert_eq!(message.id(), echo.id());
    assert_eq!(message.server_time, echo.server_time);
    assert_eq!(message.origin(), Origin::Live);
    assert!(!message.is_private());
    assert!(sent.fallbacks.is_empty());
    alice.expect_none(Event::as_message).await?;
    bob.expect_none(Event::as_message).await?;
    Ok(Completion::Completed)
}

/// a channel NOTICE carries server time and matching sender metadata
#[scenario(
    requires(caps = ["echo-message", "labeled-response", "message-tags", "server-time"]),
    covers = ["base.notice.channel", "ircv3.server-time"]
)]
async fn server_time(context: Scenario) -> ScenarioResult {
    let (mut alice, mut bob, channel) = fixture_pair(&context).await?;
    let sent = alice.notice(&channel, "maintenance at midnight").await?;
    let echo = alice.wait_for(Event::as_message).await?;
    let message = bob.wait_for(Event::as_message).await?;
    assert_eq!(echo.kind(), MessageKind::Notice);
    assert_eq!(echo.id(), sent.id.as_ref());
    assert_eq!(echo.server_time, sent.time);
    assert_eq!(echo.origin(), Origin::Echo);
    assert_eq!(message.kind(), MessageKind::Notice);
    assert_eq!(
        message.source().as_ref().and_then(Source::nick),
        Some(alice.nick().as_ref())
    );
    assert_eq!(message.target(), channel);
    assert_eq!(message.raw_text(), "maintenance at midnight");
    assert_eq!(message.id(), sent.id.as_ref());
    assert_eq!(message.server_time, sent.time);
    assert_eq!(message.id(), echo.id());
    assert_eq!(message.server_time, echo.server_time);
    assert!(message.tags().contains("time"));
    assert_eq!(message.origin(), Origin::Live);
    assert!(sent.fallbacks.is_empty());
    alice.expect_none(Event::as_message).await?;
    bob.expect_none(Event::as_message).await?;
    Ok(Completion::Completed)
}

/// direct PRIVMSG and NOTICE traffic has distinct sender and recipient views
#[scenario(
    requires(caps = ["echo-message", "labeled-response", "message-tags", "server-time"]),
    covers = [
        "base.privmsg.user-dm",
        "base.notice.user",
        "core.dm.privmsg",
        "core.dm.notice"
    ]
)]
async fn direct_messages(context: Scenario) -> ScenarioResult {
    let mut alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;

    let sent = alice.privmsg(bob.nick(), "private hello").await?;
    let echo = alice.wait_for(Event::as_message).await?;
    let received = bob.wait_for(Event::as_message).await?;
    assert_eq!(echo.origin(), Origin::Echo);
    assert_eq!(received.origin(), Origin::Live);
    assert_eq!(received.kind(), MessageKind::Privmsg);
    assert_eq!(received.target(), bob.nick().as_ref());
    assert_eq!(received.reply_target(), alice.nick().as_ref());
    assert_eq!(received.raw_text(), "private hello");
    assert!(received.is_private());
    assert_eq!(received.id(), sent.id.as_ref());
    assert_eq!(received.id(), echo.id());
    assert_eq!(received.server_time, sent.time);

    let sent = bob.notice(alice.nick(), "private notice").await?;
    let echo = bob.wait_for(Event::as_message).await?;
    let received = alice.wait_for(Event::as_message).await?;
    assert_eq!(echo.origin(), Origin::Echo);
    assert_eq!(received.origin(), Origin::Live);
    assert_eq!(received.kind(), MessageKind::Notice);
    assert_eq!(received.target(), alice.nick().as_ref());
    assert_eq!(received.reply_target(), bob.nick().as_ref());
    assert_eq!(received.raw_text(), "private notice");
    assert!(received.is_private());
    assert_eq!(received.id(), sent.id.as_ref());
    assert_eq!(received.id(), echo.id());
    assert_eq!(received.server_time, sent.time);
    assert_eq!(alice.state().nick(), &alice.nick());
    assert_eq!(bob.state().nick(), &bob.nick());
    alice.expect_none(Event::as_message).await?;
    bob.expect_none(Event::as_message).await?;
    Ok(Completion::Completed)
}

/// delivers a client-only typing tag
#[scenario(
    requires(caps = ["message-tags"]),
    covers = ["ircv3.message-tags.typing-direct"]
)]
async fn message_tags(context: Scenario) -> ScenarioResult {
    let alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();

    alice
        .typing(bob_nick.clone(), Typing::Active)
        .await
        .map_err(Failed::from)?;
    let typing = bob.wait_for(Event::as_typing_changed).await?;

    assert_eq!(
        typing.source.as_ref().and_then(Source::nick),
        Some(alice_nick.as_ref())
    );
    assert_eq!(typing.target.as_ref(), bob_nick.as_ref());
    assert_eq!(typing.state, Typing::Active);

    alice
        .typing(bob_nick, Typing::Active)
        .await
        .map_err(Failed::from)?;
    bob.expect_none(Event::as_typing_changed).await?;
    Ok(Completion::Completed)
}

/// preserves identities for plain and multiline messages
#[scenario(requires(caps = [
    "batch",
    "draft/multiline",
    "echo-message",
    "labeled-response",
    "message-tags"
]))]
async fn draft_multiline(context: Scenario) -> ScenarioResult {
    let (mut receiver, mut sender, channel) = joined_pair(&context).await?;
    let plain_text = format!("plain message from {}", sender.welcome.nick);
    let plain = sender
        .client
        .privmsg(&channel, &plain_text)
        .await
        .map_err(Failed::from)?;
    let Some(plain_id) = plain.id else {
        return Err("plain message echo did not include a message identifier".into());
    };
    context
        .expect(
            &mut receiver.events,
            |event| matches!(event, Event::Message(message) if message.raw_text() == plain_text && message.id() == Some(&plain_id)),
            EXPECT_TIMEOUT,
        )
        .await?;

    let multiline_text = "first deployment line\nsecond deployment line";
    let multiline = sender
        .client
        .privmsg(&channel, multiline_text)
        .await
        .map_err(Failed::from)?;
    let Some(multiline_id) = multiline.id else {
        return Err("multiline echo did not include a message identifier".into());
    };
    context
        .expect(
            &mut receiver.events,
            |event| matches!(event, Event::Message(message) if message.raw_text() == multiline_text && message.id() == Some(&multiline_id)),
            EXPECT_TIMEOUT,
        )
        .await?;

    receiver
        .client
        .privmsg(&channel, "reply through the multiline id")
        .reply_to(&multiline_id)
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut sender.events,
            |event| matches!(event, Event::Message(message) if message.raw_text() == "reply through the multiline id" && message.reply_to() == Some(multiline_id.as_ref())),
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
#[scenario(requires(caps = [
    "batch",
    "draft/message-redaction",
    "draft/multiline",
    "echo-message",
    "labeled-response"
]))]
async fn draft_message_redaction(context: Scenario) -> ScenarioResult {
    let caps = [
        "batch",
        "draft/message-redaction",
        "draft/multiline",
        "echo-message",
        "labeled-response",
    ];
    let sender_nick = context.unique();
    let receiver_nick = context.unique();
    let channel = format!("#{}", context.unique());
    let mut sender = context.client_caps(&sender_nick, &caps).await?;
    join(&context, &mut sender, &channel).await?;
    let mut receiver = context.client_caps(&receiver_nick, &caps).await?;
    join(&context, &mut receiver, &channel).await?;
    let sent = sender
        .client
        .privmsg(&channel, "message to\nredact")
        .await
        .map_err(Failed::from)?;
    let Some(id) = sent.id else {
        return Err("multiline echo did not include a message identifier".into());
    };
    context
        .expect(
            &mut receiver.events,
            |event| matches!(event, Event::Message(message) if message.raw_text() == "message to\nredact" && message.id() == Some(&id)),
            EXPECT_TIMEOUT,
        )
        .await?;
    sender
        .client
        .redact(papo::Target::from(&channel), &id, "conformance redaction")
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
