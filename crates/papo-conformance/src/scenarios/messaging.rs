use libtest_mimic::Failed;
use papo::{Event, MessageKind, Origin, Source};
use papo_conformance_macros::scenario;
use papo_proto::Typing;
use papo_testing::Peer;

use super::support::{EXPECT_TIMEOUT, close_pair, join, joined_pair};
use crate::{Scenario, ScenarioResult};

async fn fixture_pair(context: &Scenario) -> papo_testing::Result<(Peer, Peer, String)> {
    let mut alice = context.connect("alice").await?;
    let bob = context.connect("bob").await?;
    let channel = context.channel_name();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
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
async fn echo_message(context: Scenario) -> papo_testing::Result<()> {
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
    Ok(())
}

/// a channel NOTICE carries server time and matching sender metadata
#[scenario(
    requires(caps = ["echo-message", "labeled-response", "message-tags", "server-time"]),
    covers = ["base.notice.channel", "ircv3.server-time"]
)]
async fn server_time(context: Scenario) -> papo_testing::Result<()> {
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
    Ok(())
}

/// direct PRIVMSG and NOTICE traffic has distinct sender and recipient views
#[scenario(
    requires(caps = ["echo-message", "labeled-response", "message-tags", "server-time"]),
    covers = [
        "base.privmsg.user-dm",
        "base.notice.user",
        "core.dm.privmsg",
        "core.dm.notice",
        "core.dm.echo-self"
    ]
)]
async fn direct_messages(context: Scenario) -> papo_testing::Result<()> {
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

    let sent = alice.privmsg(alice.nick(), "note to self").await?;
    let first = alice.wait_for(Event::as_message).await?;
    let second = alice.wait_for(Event::as_message).await?;
    assert!(matches!(
        (first.origin(), second.origin()),
        (Origin::Echo, Origin::OtherSession) | (Origin::OtherSession, Origin::Echo)
    ));
    for received in [&first, &second] {
        assert_eq!(received.target(), alice.nick().as_ref());
        assert_eq!(received.raw_text(), "note to self");
        assert_eq!(received.id(), sent.id.as_ref());
        assert!(received.is_private());
    }
    assert_eq!(alice.state().nick(), &alice.nick());
    assert_eq!(bob.state().nick(), &bob.nick());
    alice.expect_none(Event::as_message).await?;
    bob.expect_none(Event::as_message).await?;
    Ok(())
}

/// status-prefixed channel messages reach only members with the requested privilege
#[scenario(
    requires(
        caps = ["echo-message", "labeled-response"],
        isupport = ["STATUSMSG"]
    ),
    covers = ["base.privmsg.statusmsg", "base.isupport.statusmsg-token"]
)]
async fn status_message(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;
    let mut charlie = s.connect("charlie").await?;
    charlie.join(&channel).await?;
    let charlie_nick = charlie.nick();
    alice
        .wait_for(|event| {
            event
                .as_joined()
                .filter(|joined| joined.nick == charlie_nick)
        })
        .await?;

    alice.mode(&channel).voice(bob.nick()).await?;
    let sent = alice
        .privmsg(format!("+{channel}"), "voiced members only")
        .await?;
    let echo = alice.wait_for(Event::as_message).await?;
    let received = bob.wait_for(Event::as_message).await?;
    for message in [&echo, &received] {
        assert_eq!(message.target(), channel);
        assert_eq!(message.status.as_deref(), Some(['+'].as_slice()));
        assert_eq!(message.raw_text(), "voiced members only");
        assert_eq!(message.id(), sent.id.as_ref());
    }
    assert_eq!(echo.origin(), Origin::Echo);
    assert_eq!(received.origin(), Origin::Live);
    assert!(
        bob.state()
            .channel(&channel)
            .and_then(|members| members.member(bob.nick()))
            .is_some_and(|member| member.prefixes().contains(&'+'))
    );
    charlie.expect_none(Event::as_message).await?;
    Ok(())
}

/// CTCP actions and automatic private query replies retain their typed forms
#[scenario(
    requires(caps = ["echo-message", "labeled-response"]),
    covers = [
        "base.ctcp.action",
        "base.ctcp.version-query-reply",
        "base.ctcp.ping-roundtrip"
    ]
)]
async fn ctcp(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;

    alice.action(&channel, "waves hello").await?;
    let echo = alice.wait_for(Event::as_message).await?;
    let action = bob.wait_for(Event::as_message).await?;
    for message in [&echo, &action] {
        assert_eq!(message.kind(), MessageKind::Action);
        assert_eq!(message.target(), channel);
        assert_eq!(message.raw_text(), "\u{1}ACTION waves hello\u{1}");
        let ctcp = message.ctcp().expect("ACTION has a CTCP representation");
        assert_eq!(ctcp.command(), "ACTION");
        assert_eq!(ctcp.params(), Some("waves hello"));
    }

    alice
        .send_command("PRIVMSG", [bob.nick().as_ref(), "\u{1}VERSION\u{1}"])
        .await?;
    let query = bob
        .wait_for(|event| {
            event
                .as_message()
                .filter(|message| message.kind() == MessageKind::Ctcp)
        })
        .await?;
    assert_eq!(query.ctcp().map(|ctcp| ctcp.command()), Some("VERSION"));
    let reply = alice
        .wait_for(|event| {
            event.as_message().filter(|message| {
                message.kind() == MessageKind::CtcpReply
                    && message.source().as_ref().and_then(Source::nick) == Some(bob.nick().as_ref())
            })
        })
        .await?;
    let version = reply.ctcp().expect("VERSION reply is CTCP");
    assert_eq!(version.command(), "VERSION");
    assert!(
        version
            .params()
            .is_some_and(|params| params.starts_with("papo "))
    );

    let mut carol = s.connect("carol").await?;
    alice
        .send_command(
            "PRIVMSG",
            [carol.nick().as_ref(), "\u{1}PING 1735689600\u{1}"],
        )
        .await?;
    let query = carol
        .wait_for(|event| {
            event
                .as_message()
                .filter(|message| message.kind() == MessageKind::Ctcp)
        })
        .await?;
    assert_eq!(
        query.ctcp().and_then(|ctcp| ctcp.params()),
        Some("1735689600")
    );
    let reply = alice
        .wait_for(|event| {
            event.as_message().filter(|message| {
                message.kind() == MessageKind::CtcpReply
                    && message.source().as_ref().and_then(Source::nick)
                        == Some(carol.nick().as_ref())
                    && message.ctcp().map(|ctcp| ctcp.command()) == Some("PING")
            })
        })
        .await?;
    assert_eq!(
        reply.ctcp().and_then(|ctcp| ctcp.params()),
        Some("1735689600")
    );
    Ok(())
}

/// replies and reaction changes retain the referenced message identifier
#[scenario(
    requires(caps = ["echo-message", "labeled-response", "message-tags"]),
    covers = ["tag.reply", "tag.react"]
)]
async fn replies_and_reactions(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;
    alice.privmsg(&channel, "ship release 2.19").await?;
    alice.wait_for(Event::as_message).await?;
    let original = bob.wait_for(Event::as_message).await?;
    let original_id = original.id().expect("message has an identifier").clone();

    bob.reply(&original, "acknowledged").await?;
    bob.wait_for(Event::as_message).await?;
    let reply = alice.wait_for(Event::as_message).await?;
    assert_eq!(reply.raw_text(), "acknowledged");
    assert_eq!(reply.reply_to(), Some(original_id.as_ref()));
    assert_eq!(reply.target(), channel);

    bob.react_to(&original, "👍").await?;
    let added = alice.wait_for(Event::as_reacted).await?;
    assert_eq!(added.target.as_ref(), channel);
    assert_eq!(added.message, original_id);
    assert_eq!(added.reaction, "👍");
    assert!(!added.removed);
    assert_eq!(
        added.source.as_ref().and_then(Source::nick),
        Some(bob.nick().as_ref())
    );

    bob.unreact(&channel, original_id.clone(), "👍").await?;
    let removed = alice.wait_for(Event::as_reacted).await?;
    assert_eq!(removed.target.as_ref(), channel);
    assert_eq!(removed.message, original_id);
    assert_eq!(removed.reaction, "👍");
    assert!(removed.removed);
    Ok(())
}

/// delivers a client-only typing tag
#[scenario(
    requires(caps = ["message-tags"]),
    covers = ["ircv3.message-tags.tagmsg", "tag.typing"]
)]
async fn message_tags(context: Scenario) -> papo_testing::Result<()> {
    let alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();

    alice.typing(bob_nick.clone(), Typing::Active).await?;
    let typing = bob.wait_for(Event::as_typing_changed).await?;

    assert_eq!(
        typing.source.as_ref().and_then(Source::nick),
        Some(alice_nick.as_ref())
    );
    assert_eq!(typing.target.as_ref(), bob_nick.as_ref());
    assert_eq!(typing.state, Typing::Active);

    alice.typing(bob_nick.clone(), Typing::Active).await?;
    bob.expect_none(Event::as_typing_changed).await?;
    alice.typing(bob_nick.clone(), Typing::Paused).await?;
    assert_eq!(
        bob.wait_for(Event::as_typing_changed).await?.state,
        Typing::Paused
    );
    alice.typing(bob_nick, Typing::Done).await?;
    assert_eq!(
        bob.wait_for(Event::as_typing_changed).await?.state,
        Typing::Done
    );
    Ok(())
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
