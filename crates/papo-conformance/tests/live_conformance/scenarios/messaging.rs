use papo::{Error, Event, Fallback, Feature, MessageKind, Origin, Refusal, UnsupportedReason};
use papo_conformance_macros::scenario;
use papo_proto::Typing;

use super::support::{conversation_name, message_text};
use crate::{Peer, Scenario};

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
    assert_eq!(joined.authorship, papo::Authorship::Other);
    for peer in [&alice, &bob] {
        let state = peer.state();
        let members = state.channel(&channel).expect("joined channel is in state");
        assert!(members.is_synced());
        assert!(members.member(alice.nick()).is_some());
        assert!(members.member(bob.nick()).is_some());
    }
    Ok((alice, bob, channel))
}

fn is_private(message: &papo::Message) -> bool {
    matches!(message.conversation, papo::Conversation::Private(_))
}

/// a channel PRIVMSG confirms through its future and reaches one recipient
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
    let message = bob
        .wait_for(|event| message_text(event, "hello from alice"))
        .await?;
    assert_eq!(message.kind, MessageKind::Privmsg);
    assert_eq!(
        crate::source_nick(&message.actor),
        Some(alice.nick().as_ref())
    );
    assert_eq!(
        conversation_name(&message.conversation),
        Some(channel.as_str())
    );
    assert_eq!(message.raw_text, "hello from alice");
    assert_eq!(message.text, "hello from alice");
    assert_eq!(message.msgid.as_ref(), sent.msgid.as_ref());
    assert_eq!(Some(message.time), sent.server_time);
    assert_eq!(message.origin, Origin::Live);
    assert!(!is_private(&message));
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
    let message = bob
        .wait_for(|event| message_text(event, "maintenance at midnight"))
        .await?;
    assert_eq!(message.kind, MessageKind::Notice);
    assert_eq!(
        crate::source_nick(&message.actor),
        Some(alice.nick().as_ref())
    );
    assert_eq!(
        conversation_name(&message.conversation),
        Some(channel.as_str())
    );
    assert_eq!(message.raw_text, "maintenance at midnight");
    assert_eq!(message.msgid.as_ref(), sent.msgid.as_ref());
    assert_eq!(Some(message.time), sent.server_time);
    assert!(message.tags.contains("time"));
    assert_eq!(message.origin, Origin::Live);
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
    let received = bob
        .wait_for(|event| message_text(event, "private hello"))
        .await?;
    assert_eq!(received.origin, Origin::Live);
    assert_eq!(received.kind, MessageKind::Privmsg);
    assert_eq!(
        conversation_name(&received.conversation),
        Some(alice.nick().as_ref())
    );
    assert_eq!(received.return_address(), alice.nick().as_ref());
    assert_eq!(received.raw_text, "private hello");
    assert!(is_private(&received));
    assert_eq!(received.msgid.as_ref(), sent.msgid.as_ref());
    assert_eq!(Some(received.time), sent.server_time);

    let sent = bob.notice(alice.nick(), "private notice").await?;
    let received = alice
        .wait_for(|event| message_text(event, "private notice"))
        .await?;
    assert_eq!(received.origin, Origin::Live);
    assert_eq!(received.kind, MessageKind::Notice);
    assert_eq!(
        conversation_name(&received.conversation),
        Some(bob.nick().as_ref())
    );
    assert_eq!(received.return_address(), bob.nick().as_ref());
    assert_eq!(received.raw_text, "private notice");
    assert!(is_private(&received));
    assert_eq!(received.msgid.as_ref(), sent.msgid.as_ref());
    assert_eq!(Some(received.time), sent.server_time);

    let sent = alice.privmsg(alice.nick(), "note to self").await?;
    let received = alice
        .wait_for(|event| message_text(event, "note to self"))
        .await?;
    assert_eq!(received.origin, Origin::Live);
    assert_eq!(received.authorship, papo::Authorship::Own);
    assert_eq!(
        conversation_name(&received.conversation),
        Some(alice.nick().as_ref())
    );
    assert_eq!(received.raw_text, "note to self");
    assert_eq!(received.msgid.as_ref(), sent.msgid.as_ref());
    assert!(is_private(&received));
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
    assert_eq!(threaded.msgid.as_ref(), sent.msgid.as_ref());

    let unavailable = papo_test_server::unique("missing-target");
    let Error::Server(refusal) = alice
        .privmsg(&unavailable, "this cannot be delivered")
        .await
        .expect_err("a nonexistent target must be refused")
    else {
        panic!("a nonexistent target did not return a server refusal")
    };
    assert_eq!(refusal.refusal(), Refusal::NoSuchNick);
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
    let account = crate::scenarios::support::logged_in_account(&desktop.state()).clone();
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
        .wait_for(|event| event.as_message().filter(|message| message.text == text))
        .await?;
    assert_eq!(other.origin, Origin::Live);
    assert!(other.is_live());
    assert_eq!(other.authorship, papo::Authorship::Own);
    desktop
        .expect_none(|event| event.as_message().filter(|message| message.text == text))
        .await?;
    Ok(())
}

/// a session without echo-message receives another session's message as its own
#[scenario(covers = ["core.multiclient.own-message-without-echo"])]
async fn own_message_without_echo(s: Scenario) -> papo_testing::Result<()> {
    let (desktop, account_key) = s.connect_as_account_with_key("alice").await?;
    let channel = s.channel_name();
    desktop.join(&channel).await?;
    let mut phone = s
        .connect_with(
            s.configure_account_session("alice-phone", account_key)?
                .without_caps([papo::cap::ECHO_MESSAGE]),
        )
        .await?;
    assert!(phone.state().caps().get(&papo::cap::ECHO_MESSAGE).is_none());

    if phone.state().channel(&channel).is_none() {
        phone.join(&channel).await?;
    }
    let text = "message from another logged-in session";
    desktop.privmsg(&channel, text).await?;
    let message = phone
        .wait_for(|event| event.as_message().filter(|message| message.text == text))
        .await?;
    assert_eq!(message.origin, Origin::Live);
    assert_eq!(message.authorship, papo::Authorship::Own);
    assert_eq!(
        conversation_name(&message.conversation),
        Some(channel.as_str())
    );
    phone
        .expect_none(|event| event.as_message().filter(|message| message.text == text))
        .await?;
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
    let received = bob
        .wait_for(|event| message_text(event, "voiced members only"))
        .await?;
    assert_eq!(
        conversation_name(&received.conversation),
        Some(channel.as_str())
    );
    assert_eq!(received.status.as_deref(), Some(['+'].as_slice()));
    assert_eq!(received.raw_text, "voiced members only");
    assert_eq!(received.msgid.as_ref(), sent.msgid.as_ref());
    assert_eq!(received.origin, Origin::Live);
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
    let received_by_bob = bob
        .wait_for(|event| message_text(event, "voiced members only"))
        .await?;
    let received_by_alice = alice
        .wait_for(|event| message_text(event, "voiced members only"))
        .await?;
    for message in [&received_by_bob, &received_by_alice] {
        assert_eq!(
            conversation_name(&message.conversation),
            Some(channel.as_ref())
        );
        assert_eq!(message.status.as_deref(), Some(['+'].as_slice()));
        assert_eq!(message.raw_text, "voiced members only");
        assert_eq!(message.msgid.as_ref(), sent.msgid.as_ref());
    }
    assert_eq!(received_by_bob.origin, Origin::Live);
    assert_eq!(received_by_alice.origin, Origin::Live);
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

    alice.action(&channel, "waves hello").await?;
    let action = bob
        .wait_for(|event| message_text(event, "waves hello"))
        .await?;
    assert_eq!(action.kind, MessageKind::Action);
    assert_eq!(
        conversation_name(&action.conversation),
        Some(channel.as_str())
    );
    assert_eq!(action.raw_text, "waves hello");

    alice
        .send_raw(papo::Line::new(
            "PRIVMSG",
            [bob.nick().as_ref(), "\u{1}VERSION\u{1}"],
        ))
        .await?;
    let query = bob
        .wait_for(|event| event.as_ctcp().filter(|ctcp| ctcp.command == "VERSION"))
        .await?;
    assert!(!query.reply);
    let reply = alice
        .wait_for(|event| {
            event.as_ctcp().filter(|ctcp| {
                ctcp.reply && crate::source_nick(&ctcp.actor) == Some(bob.nick().as_ref())
            })
        })
        .await?;
    assert_eq!(reply.command, "VERSION");
    assert!(
        reply
            .params
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
        .wait_for(|event| event.as_ctcp().filter(|ctcp| ctcp.command == "PING"))
        .await?;
    assert_eq!(query.params.as_deref(), Some("1735689600"));
    let reply = alice
        .wait_for(|event| {
            event.as_ctcp().filter(|ctcp| {
                ctcp.reply
                    && crate::source_nick(&ctcp.actor) == Some(carol.nick().as_ref())
                    && ctcp.command == "PING"
            })
        })
        .await?;
    assert_eq!(reply.params.as_deref(), Some("1735689600"));
    Ok(())
}

/// leaves channel CTCP queries to the application, which can reply privately
#[scenario(covers = [
    "base.ctcp.channel-query-private-reply",
    "security.ctcp.private-target-only"
])]
async fn ctcp_channel_query(s: Scenario) -> papo_testing::Result<()> {
    if std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into()) != "unrealircd"
    {
        return s.skip("the selected profile blocks channel CTCP by default");
    }
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;
    alice
        .send_raw(papo::Line::new("PRIVMSG", [&channel, "\u{1}VERSION\u{1}"]))
        .await?;
    let query = bob
        .wait_for(|event| {
            event.as_ctcp().filter(|ctcp| {
                conversation_name(&ctcp.conversation) == Some(channel.as_str())
                    && ctcp.command == "VERSION"
            })
        })
        .await?;
    assert!(matches!(query.conversation, papo::Conversation::Channel(_)));
    alice
        .expect_none(|event| {
            event.as_ctcp().filter(|ctcp| {
                ctcp.reply && crate::source_nick(&ctcp.actor) == Some(bob.nick().as_ref())
            })
        })
        .await?;
    bob.ctcp_reply(&query, "papo conformance client").await?;
    let reply = alice
        .wait_for(|event| {
            event.as_ctcp().filter(|ctcp| {
                ctcp.reply
                    && conversation_name(&ctcp.conversation) == Some(bob.nick().as_ref())
                    && crate::source_nick(&ctcp.actor) == Some(bob.nick().as_ref())
            })
        })
        .await?;
    assert!(matches!(reply.conversation, papo::Conversation::Private(_)));
    Ok(())
}

/// never answers a CTCP query replayed from soju's private-message backlog
#[scenario(
    requires(caps = ["batch", "draft/chathistory"]),
    covers = ["security.ctcp.live-only"]
)]
async fn ctcp_soju_backlog(s: Scenario) -> papo_testing::Result<()> {
    if std::env::var("PAPO_TEST_PROFILE").as_deref() != Ok("soju-ergo") {
        return s.skip("the selected profile does not replay a soju backlog");
    }
    let mut alice = s.connect("alice").await?;
    let (mut bob, network) = s.connect_as_account_with_key("bob").await?;
    let bob_nick = bob.nick();
    bob.quit("detach before backlog delivery").await?;
    bob.wait_for(Event::as_disconnected).await?;

    alice
        .send_raw(papo::Line::new(
            "PRIVMSG",
            [bob_nick.as_ref(), "\u{1}VERSION\u{1}"],
        ))
        .await?;
    alice
        .expect_none(|event| {
            event.as_ctcp().filter(|ctcp| {
                ctcp.reply && crate::source_nick(&ctcp.actor) == Some(bob_nick.as_ref())
            })
        })
        .await?;

    let phone = s.connect_account_session("bob-phone", network).await?;
    let history = phone
        .history(alice.nick().as_ref())
        .latest()
        .limit(10)
        .await?;
    assert!(history.items.iter().any(|item| matches!(item,
        papo::Event::Other(line)
            if line.as_ref().param(1) == Some("\u{1}VERSION\u{1}"))));
    alice
        .expect_none(|event| {
            event.as_ctcp().filter(|ctcp| {
                ctcp.reply && crate::source_nick(&ctcp.actor) == Some(bob_nick.as_ref())
            })
        })
        .await?;
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
    alice.mode(&channel).add('C').await?;
    bob.send_raw(papo::Line::new("PRIVMSG", [&channel, "\u{1}VERSION\u{1}"]))
        .await?;
    alice
        .expect_none(|event| {
            event.as_ctcp().filter(|ctcp| {
                conversation_name(&ctcp.conversation) == Some(channel.as_str())
                    && ctcp.command == "VERSION"
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
    let original = bob
        .wait_for(|event| message_text(event, "ship release 2.19"))
        .await?;
    let original_id = original
        .msgid
        .as_ref()
        .expect("message has an identifier")
        .clone();

    let sent = bob.reply(&original, "acknowledged").await?;
    if sent.fallbacks.contains(&Fallback::ReplyTagDropped) {
        return s.skip("server denies both supported reply-tag aliases");
    }
    let reply = alice
        .wait_for(|event| message_text(event, "acknowledged"))
        .await?;
    assert_eq!(reply.raw_text, "acknowledged");
    assert_eq!(reply.reply_to.as_ref(), Some(&original_id));
    assert_eq!(
        conversation_name(&reply.conversation),
        Some(channel.as_str())
    );

    match bob.react(&original, "👍").await {
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
    assert_eq!(
        conversation_name(&added.conversation),
        Some(channel.as_str())
    );
    assert_eq!(added.status, None);
    assert_eq!(added.message, original_id);
    assert_eq!(added.reaction, "👍");
    assert!(!added.removed);
    assert_eq!(crate::source_nick(&added.actor), Some(bob.nick().as_ref()));

    bob.unreact((&channel, original_id.clone()), "👍").await?;
    let removed = alice.wait_for(Event::as_reacted).await?;
    assert_eq!(
        conversation_name(&removed.conversation),
        Some(channel.as_str())
    );
    assert_eq!(removed.status, None);
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

    assert_eq!(crate::source_nick(&typing.actor), Some(alice_nick.as_ref()));
    assert_eq!(
        conversation_name(&typing.conversation),
        Some(alice_nick.as_ref())
    );
    assert_eq!(typing.status, None);
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
        crate::source_nick(&typing.actor),
        Some(alice.nick().as_ref())
    );
    assert_eq!(
        conversation_name(&typing.conversation),
        Some(channel.as_ref())
    );
    assert_eq!(typing.status, Some(vec!['@']));
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
    assert_eq!(received.kind, MessageKind::Privmsg);
    assert_eq!(
        conversation_name(&received.conversation),
        Some(channel.as_str())
    );
    assert_eq!(received.raw_text, text);
    assert_eq!(received.text, text);
    assert_eq!(received.msgid.as_ref(), sent.msgid.as_ref());
    assert_eq!(Some(received.time), sent.server_time);
    assert_eq!(received.origin, Origin::Live);
    bob.expect_none(|event| message_text(event, text)).await?;
    Ok(())
}

/// delivers UTF-8 channel text without replacement
#[scenario(
    requires(isupport = ["UTF8ONLY"]),
    covers = ["ircv3.utf8only"]
)]
async fn utf8only(s: Scenario) -> papo_testing::Result<()> {
    let (alice, mut bob, channel) = fixture_pair(&s).await?;
    let text = "Olá, IRC 世界";
    let sent = alice.privmsg(&channel, text).await?;
    let received = bob.wait_for(|event| message_text(event, text)).await?;
    assert_eq!(received.raw_text, text);
    assert_eq!(received.text, text);
    assert_eq!(received.msgid.as_ref(), sent.msgid.as_ref());
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
    let (alice, mut bob, channel) = fixture_pair(&s).await?;
    let sent = alice.privmsg(&channel, "message to\nredact").await?;
    let received = bob
        .wait_for(|event| message_text(event, "message to\nredact"))
        .await?;
    let id = received
        .msgid
        .as_ref()
        .expect("message has an identifier")
        .clone();
    assert_eq!(Some(&id), sent.msgid.as_ref());

    alice.redact((&channel, id.clone()), "superseded").await?;
    let redacted = bob.wait_for(Event::as_redacted).await?;
    assert_eq!(
        conversation_name(&redacted.conversation),
        Some(channel.as_str())
    );
    assert_eq!(redacted.status, None);
    assert_eq!(redacted.message, id);
    assert_eq!(redacted.reason.as_deref(), Some("superseded"));
    assert_eq!(
        crate::source_nick(&redacted.actor),
        Some(alice.nick().as_ref())
    );
    bob.expect_none(Event::as_redacted).await?;

    let Error::Server(refusal) = alice
        .redact((&channel, "missing-message-id"), "unknown message")
        .await
        .expect_err("an unknown message identifier must be refused")
    else {
        panic!("an unknown message identifier did not return a server refusal")
    };
    assert_eq!(refusal.refusal(), Refusal::UnknownMessage);
    assert_eq!(refusal.code(), "UNKNOWN_MSGID");
    bob.expect_none(Event::as_redacted).await?;
    Ok(())
}
