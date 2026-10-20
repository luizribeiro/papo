use papo::{
    Error, Event, Fallback, Feature, MessageKind, Origin, Refusal, Source, UnsupportedReason,
};
use papo_conformance_macros::scenario;
use papo_proto::Typing;
use papo_testing::Peer;

use super::support::message_text;
use crate::Scenario;

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
    covers = [
        "base.privmsg.channel",
        "ircv3.echo-message",
        "ircv3.message-tags.msgid"
    ]
)]
async fn echo_message(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;
    let sent = alice.privmsg(&channel, "hello from alice").await?;
    let echo = alice
        .wait_for(|event| message_text(event, "hello from alice"))
        .await?;
    let message = bob
        .wait_for(|event| message_text(event, "hello from alice"))
        .await?;
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
    alice
        .expect_none(|event| message_text(event, "hello from alice"))
        .await?;
    bob.expect_none(|event| message_text(event, "hello from alice"))
        .await?;
    Ok(())
}

/// a channel NOTICE carries server time and matching sender metadata
#[scenario(
    requires(caps = ["echo-message", "labeled-response", "message-tags", "server-time"]),
    covers = ["base.notice.channel", "ircv3.server-time"]
)]
async fn server_time(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;
    let sent = alice.notice(&channel, "maintenance at midnight").await?;
    let echo = alice
        .wait_for(|event| message_text(event, "maintenance at midnight"))
        .await?;
    let message = bob
        .wait_for(|event| message_text(event, "maintenance at midnight"))
        .await?;
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
    alice
        .expect_none(|event| message_text(event, "maintenance at midnight"))
        .await?;
    bob.expect_none(|event| message_text(event, "maintenance at midnight"))
        .await?;
    Ok(())
}

/// direct PRIVMSG and NOTICE traffic has distinct sender and recipient views
#[scenario(
    requires(caps = ["echo-message", "labeled-response", "message-tags", "server-time"]),
    covers = [
        "base.privmsg.user-dm",
        "base.notice.user",
        "base.privmsg.nonexistent-target",
        "core.dm.privmsg",
        "core.dm.notice",
        "core.dm.echo-self",
        "tag.channel-context"
    ]
)]
async fn direct_messages(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;

    let sent = alice.privmsg(bob.nick(), "private hello").await?;
    let echo = alice
        .wait_for(|event| message_text(event, "private hello"))
        .await?;
    let received = bob
        .wait_for(|event| message_text(event, "private hello"))
        .await?;
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
    let echo = bob
        .wait_for(|event| message_text(event, "private notice"))
        .await?;
    let received = alice
        .wait_for(|event| message_text(event, "private notice"))
        .await?;
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
    let first = alice
        .wait_for(|event| message_text(event, "note to self"))
        .await?;
    let second = alice
        .wait_for(|event| message_text(event, "note to self"))
        .await?;
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

    let channel = s.channel_name();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let channel = alice.state().casemapping().channel(channel);
    let sent = alice
        .privmsg(bob.nick(), "private thread reply")
        .channel_context(&channel)
        .await?;
    assert!(!sent.fallbacks.contains(&Fallback::ContextTagDropped));
    let threaded = bob
        .wait_for(|event| message_text(event, "private thread reply"))
        .await?;
    assert_eq!(threaded.channel_context.as_ref(), Some(&channel));
    assert_eq!(threaded.id(), sent.id.as_ref());

    let unavailable = papo_test_server::unique("missing-target");
    let Error::Server(refusal) = alice
        .privmsg(&unavailable, "this cannot be delivered")
        .await
        .expect_err("a nonexistent target must be refused")
    else {
        panic!("a nonexistent target did not return a server refusal")
    };
    assert_eq!(refusal.kind(), Refusal::NoSuchNick);
    assert_eq!(refusal.code(), "401");
    alice
        .expect_none(|event| message_text(event, "note to self"))
        .await?;
    bob.expect_none(|event| message_text(event, "note to self"))
        .await?;
    Ok(())
}

/// synchronizes channel state and messages across sessions of one account
#[scenario(covers = [
    "core.multiclient.fanout",
    "core.multiclient.state-sync-on-attach"
])]
async fn multiclient(s: Scenario) -> papo_testing::Result<()> {
    let profile = std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into());
    if !matches!(profile.as_str(), "ergo-default" | "ergo-persistent") {
        return s.skip("the selected profile does not expose account multiclient attachment");
    }
    let mut desktop = s.connect_as_account("alice").await?;
    let account = desktop.state().account().unwrap().clone();
    let channel = s.channel_name();
    desktop.join(&channel).await?;
    let mut phone = s
        .connect_account_session("alice-phone", account.as_ref())
        .await?;
    phone
        .wait_for(|event| {
            event
                .as_channel_synced()
                .filter(|synced| synced.channel.as_ref() == channel)
        })
        .await?;
    let phone_state = phone.state();
    let attached = phone_state
        .channel(&channel)
        .expect("another account session inherits joined channels");
    assert!(attached.is_synced());

    let text = "message from the desktop session";
    desktop.privmsg(&channel, text).await?;
    let other = phone
        .wait_for(|event| event.as_message().filter(|message| message.text() == text))
        .await?;
    assert_eq!(other.origin(), Origin::OtherSession);
    assert!(!other.is_live());
    let echo = desktop
        .wait_for(|event| event.as_message().filter(|message| message.text() == text))
        .await?;
    assert_eq!(echo.origin(), Origin::Echo);
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
    let echo = alice
        .wait_for(|event| message_text(event, "voiced members only"))
        .await?;
    let received = bob
        .wait_for(|event| message_text(event, "voiced members only"))
        .await?;
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
    charlie
        .expect_none(|event| message_text(event, "voiced members only"))
        .await?;
    Ok(())
}

/// a voice-targeted message reaches operators as higher-ranked members
#[scenario(
    requires(
        caps = ["echo-message", "labeled-response"],
        isupport = ["STATUSMSG"]
    ),
    covers = ["base.privmsg.statusmsg-or-higher"]
)]
async fn status_message_or_higher(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let mut carol = s.connect("carol").await?;
    let mut dave = s.connect("dave").await?;
    let channel = alice.state().channel_name(s.channel_name());
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    carol.join(&channel).await?;
    dave.join(&channel).await?;

    alice.mode(&channel).op(bob.nick()).await?;
    alice.mode(&channel).voice(carol.nick()).await?;
    let sent = carol
        .privmsg(format!("+{channel}"), "voiced members only")
        .await?;
    let echo = carol
        .wait_for(|event| message_text(event, "voiced members only"))
        .await?;
    let received_by_bob = bob
        .wait_for(|event| message_text(event, "voiced members only"))
        .await?;
    let received_by_alice = alice
        .wait_for(|event| message_text(event, "voiced members only"))
        .await?;
    for message in [&echo, &received_by_bob, &received_by_alice] {
        assert_eq!(message.target(), channel.as_ref());
        assert_eq!(message.status.as_deref(), Some(['+'].as_slice()));
        assert_eq!(message.raw_text(), "voiced members only");
        assert_eq!(message.id(), sent.id.as_ref());
    }
    assert_eq!(echo.origin(), Origin::Echo);
    assert_eq!(received_by_bob.origin(), Origin::Live);
    assert_eq!(received_by_alice.origin(), Origin::Live);
    dave.expect_none(|event| message_text(event, "voiced members only"))
        .await?;
    carol
        .expect_none(|event| message_text(event, "voiced members only"))
        .await?;
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
    let channel_name = alice.state().channel_name(&channel);

    alice.action(&channel, "waves hello").await?;
    let echo = alice
        .wait_for(|event| message_text(event, "\u{1}ACTION waves hello\u{1}"))
        .await?;
    let action = bob
        .wait_for(|event| message_text(event, "\u{1}ACTION waves hello\u{1}"))
        .await?;
    for message in [&echo, &action] {
        assert_eq!(message.kind(), MessageKind::Action);
        assert_eq!(channel_name, message.target());
        assert_eq!(message.raw_text(), "\u{1}ACTION waves hello\u{1}");
        let ctcp = message.ctcp().expect("ACTION has a CTCP representation");
        assert_eq!(ctcp.command(), "ACTION");
        assert_eq!(ctcp.params(), Some("waves hello"));
    }

    alice
        .send_raw(papo::Line::new(
            "PRIVMSG",
            [bob.nick().as_ref(), "\u{1}VERSION\u{1}"],
        ))
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
        .send_raw(papo::Line::new(
            "PRIVMSG",
            [carol.nick().as_ref(), "\u{1}PING 1735689600\u{1}"],
        ))
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

/// answers a channel CTCP query with a private response
#[scenario(covers = ["base.ctcp.channel-query-private-reply"])]
async fn ctcp_channel_query(s: Scenario) -> papo_testing::Result<()> {
    if std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into()) != "unrealircd"
    {
        return s.skip("the selected profile blocks channel CTCP by default");
    }
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    let channel_name = alice.state().channel_name(&channel);
    alice
        .send_raw(papo::Line::new("PRIVMSG", [&channel, "\u{1}VERSION\u{1}"]))
        .await?;
    let query = bob
        .wait_for(|event| {
            event.as_message().filter(|message| {
                channel_name == message.target()
                    && message.ctcp().map(|ctcp| ctcp.command()) == Some("VERSION")
            })
        })
        .await?;
    assert!(!query.is_private());
    let reply = alice
        .wait_for(|event| {
            event.as_message().filter(|message| {
                message.kind() == MessageKind::CtcpReply
                    && alice_nick == message.target()
                    && message.source().as_ref().and_then(Source::nick) == Some(bob_nick.as_ref())
            })
        })
        .await?;
    assert!(reply.is_private());
    Ok(())
}

/// blocks channel CTCP traffic when the server's no-CTCP mode is set
#[scenario(covers = ["base.ctcp.no-ctcp-mode"])]
async fn no_ctcp_mode(s: Scenario) -> papo_testing::Result<()> {
    let profile = std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into());
    if !matches!(profile.as_str(), "ergo-default" | "ergo-persistent") {
        return s.skip("the selected profile has no managed no-CTCP channel mode");
    }
    let (mut alice, bob, channel) = fixture_pair(&s).await?;
    let channel_name = alice.state().channel_name(&channel);
    alice.mode(&channel).add('C').await?;
    bob.send_raw(papo::Line::new("PRIVMSG", [&channel, "\u{1}VERSION\u{1}"]))
        .await?;
    alice
        .expect_none(|event| {
            event.as_message().filter(|message| {
                channel_name == message.target()
                    && message.ctcp().map(|ctcp| ctcp.command()) == Some("VERSION")
            })
        })
        .await?;
    Ok(())
}

/// replies and reaction changes retain the referenced message identifier
#[scenario(
    requires(caps = ["echo-message", "labeled-response", "message-tags"]),
    covers = [
        "ircv3.labeled-response.ack-no-reply",
        "ircv3.message-tags.clienttagdeny",
        "tag.reply",
        "tag.react"
    ]
)]
async fn replies_and_reactions(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;
    alice.privmsg(&channel, "ship release 2.19").await?;
    alice
        .wait_for(|event| message_text(event, "ship release 2.19"))
        .await?;
    let original = bob
        .wait_for(|event| message_text(event, "ship release 2.19"))
        .await?;
    let original_id = original.id().expect("message has an identifier").clone();

    let sent = bob.reply(&original, "acknowledged").await?;
    if sent.fallbacks.contains(&Fallback::ReplyTagDropped) {
        return s.skip("server denies both supported reply-tag aliases");
    }
    bob.wait_for(|event| message_text(event, "acknowledged"))
        .await?;
    let reply = alice
        .wait_for(|event| message_text(event, "acknowledged"))
        .await?;
    assert_eq!(reply.raw_text(), "acknowledged");
    assert_eq!(reply.reply_to(), Some(original_id.as_ref()));
    assert_eq!(reply.target(), channel);

    match bob.react_to(&original, "👍").await {
        Ok(()) => {}
        Err(Error::Unsupported(unsupported))
            if unsupported.feature == Feature::Reactions
                && unsupported.reason == UnsupportedReason::TagDenied =>
        {
            return s.skip("server denies its reaction client-only tag");
        }
        Err(error) => return Err(error.into()),
    }
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
async fn message_tags(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
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

/// a typing notification to an operator target reaches only channel operators
#[scenario(
    requires(caps = ["message-tags"], isupport = ["STATUSMSG"]),
    covers = ["tag.typing.status-target"]
)]
async fn typing_status_target(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let mut carol = s.connect("carol").await?;
    let mut dave = s.connect("dave").await?;
    let channel = alice.state().channel_name(s.channel_name());
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    carol.join(&channel).await?;
    dave.join(&channel).await?;
    alice.mode(&channel).op(bob.nick()).await?;

    let target = format!("@{channel}");
    alice.typing(&target, Typing::Active).await?;
    let typing = bob.wait_for(Event::as_typing_changed).await?;
    assert_eq!(
        typing.source.as_ref().and_then(Source::nick),
        Some(alice.nick().as_ref())
    );
    assert_eq!(typing.target.as_ref(), target);
    assert_eq!(typing.state, Typing::Active);
    carol.expect_none(Event::as_typing_changed).await?;
    dave.expect_none(Event::as_typing_changed).await?;
    Ok(())
}

/// multiline delivery concatenates lines while preserving message metadata
#[scenario(
    requires(caps = [
        "batch",
        "draft/multiline",
        "echo-message",
        "labeled-response",
        "message-tags"
    ]),
    covers = ["draft.multiline.concat", "tag.multiline-concat"]
)]
async fn draft_multiline(s: Scenario) -> papo_testing::Result<()> {
    let (alice, mut bob, channel) = fixture_pair(&s).await?;
    let text = "first deployment line\nsecond deployment line";
    let sent = alice.privmsg(&channel, text).await?;
    let received = bob.wait_for(|event| message_text(event, text)).await?;
    assert_eq!(received.kind(), MessageKind::Privmsg);
    assert_eq!(received.target(), channel);
    assert_eq!(received.raw_text(), text);
    assert_eq!(received.text(), text);
    assert_eq!(received.id(), sent.id.as_ref());
    assert_eq!(received.server_time, sent.time);
    assert_eq!(received.origin(), Origin::Live);
    bob.expect_none(|event| message_text(event, text)).await?;
    Ok(())
}

/// delivers UTF-8 channel text without replacement
#[scenario(
    requires(isupport = ["UTF8ONLY"]),
    covers = ["ircv3.utf8only"]
)]
async fn utf8only(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;
    let text = "Olá, IRC 世界";
    let sent = alice.privmsg(&channel, text).await?;
    let echo = alice.wait_for(|event| message_text(event, text)).await?;
    let received = bob.wait_for(|event| message_text(event, text)).await?;
    assert_eq!(echo.raw_text(), text);
    assert_eq!(received.raw_text(), text);
    assert_eq!(received.text(), text);
    assert_eq!(received.id(), sent.id.as_ref());
    Ok(())
}

/// redacts a delivered channel message
#[scenario(
    requires(caps = [
        "batch",
        "draft/message-redaction",
        "draft/multiline",
        "echo-message",
        "labeled-response"
    ]),
    covers = [
        "draft.message-redaction.redact",
        "draft.message-redaction.errors"
    ]
)]
async fn draft_message_redaction(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;
    let sent = alice.privmsg(&channel, "message to\nredact").await?;
    alice
        .wait_for(|event| message_text(event, "message to\nredact"))
        .await?;
    let received = bob
        .wait_for(|event| message_text(event, "message to\nredact"))
        .await?;
    let id = received.id().expect("message has an identifier").clone();
    assert_eq!(Some(&id), sent.id.as_ref());

    alice.redact(&channel, id.clone(), "superseded").await?;
    let redacted = bob.wait_for(Event::as_redacted).await?;
    assert_eq!(redacted.target.as_ref(), channel);
    assert_eq!(redacted.message, id);
    assert_eq!(redacted.reason.as_deref(), Some("superseded"));
    assert_eq!(
        redacted.source.as_ref().and_then(Source::nick),
        Some(alice.nick().as_ref())
    );
    bob.expect_none(Event::as_redacted).await?;

    let Error::Server(refusal) = alice
        .redact(&channel, "missing-message-id", "unknown message")
        .await
        .expect_err("an unknown message identifier must be refused")
    else {
        panic!("an unknown message identifier did not return a server refusal")
    };
    assert_eq!(refusal.kind(), Refusal::UnknownMessage);
    assert_eq!(refusal.code(), "UNKNOWN_MSGID");
    bob.expect_none(Event::as_redacted).await?;
    Ok(())
}
