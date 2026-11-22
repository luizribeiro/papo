use papo::{Error, Event, Fallback, MessageKind, Refusal};
use papo_conformance_macros::scenario;
use papo_proto::Typing;

use super::support::{
    any_message_text, assert_delivery_tags, confirmed_line, confirmed_redaction,
    conversation_is_private, conversation_name, message_text, notice_text, reaction_or_skip,
    restricted_line, restricted_message, restricted_text,
};
use crate::{Peer, Scenario};

fn name_rules(state: &papo::State) -> papo::NameRules<'_> {
    state.name_rules().expect("server advertised CASEMAPPING")
}

async fn fixture_pair(context: &Scenario) -> papo_testing::Result<(Peer, Peer, String)> {
    fixture_pair_with(context, context.configure("alice")?).await
}

async fn fixture_pair_with(
    context: &Scenario,
    alice_builder: papo::Builder,
) -> papo_testing::Result<(Peer, Peer, String)> {
    let alice = context.connect_with(alice_builder).await?;
    let bob = context.connect("bob").await?;
    joined_pair(context, alice, bob).await
}

async fn joined_pair(
    context: &Scenario,
    mut alice: Peer,
    bob: Peer,
) -> papo_testing::Result<(Peer, Peer, String)> {
    let channel = context.channel_name();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let bob_nick = bob.snapshot().nick().clone();
    let joined = alice
        .wait_for(|event| {
            event
                .as_joined()
                .filter(|joined| joined.actor.nick() == bob_nick.as_str())
        })
        .await?;
    assert_eq!(joined.channel.name().as_str(), channel);
    assert_eq!(joined.actor.nick(), bob_nick.as_str());
    assert_eq!(joined.actor_party, papo::Party::Other);
    for peer in [&alice, &bob] {
        let state = peer.snapshot();
        let members = state
            .__channel_local(&channel)
            .expect("joined channel is in state");
        assert!(members.is_synced());
        assert!(
            members
                .member(alice.snapshot().nick().as_str())
                .unwrap()
                .is_some()
        );
        assert!(
            members
                .member(bob.snapshot().nick().as_str())
                .unwrap()
                .is_some()
        );
    }
    Ok((alice, bob, channel))
}

fn is_private(message: &papo::Message) -> bool {
    conversation_is_private(message.conversation())
}

async fn assert_unverified_context(
    sender: &Peer,
    recipient: &mut Peer,
    context: &str,
    text: &'static str,
) -> papo_testing::Result<()> {
    let direct = recipient.snapshot().nick().to_string();
    let context_tag_allowed = context_tag_allowed(sender);
    let sent = sender
        .privmsg(&direct, text)
        .channel_context(context)
        .await?;
    assert_eq!(
        sent.fallbacks.contains(&Fallback::ContextTagDropped),
        !context_tag_allowed
    );
    let message = recipient
        .wait_for(|event| message_text(event, text))
        .await?;
    assert_eq!(message.channel_context(), None);
    if !context_tag_allowed {
        assert_eq!(message.tags.get("+channel-context"), None);
        assert_eq!(message.tags.get("+draft/channel-context"), None);
    } else if let Some(relayed_context) = message
        .tags
        .get("+channel-context")
        .or_else(|| message.tags.get("+draft/channel-context"))
    {
        // Servers may moderate client-only tags. If an unverified context is relayed, papo
        // must retain it as raw metadata without treating it as a verified context.
        assert_eq!(relayed_context, context);
    }
    Ok(())
}

fn context_tag_allowed(peer: &Peer) -> bool {
    let state = peer.snapshot();
    let denied = state.isupport().clienttagdeny();
    !denied.denies("+channel-context") || !denied.denies("+draft/channel-context")
}

async fn part_for_unverified_context(
    alice: &mut Peer,
    bob: &mut Peer,
    channel: &str,
) -> papo_testing::Result<()> {
    // PART is setup for the unverified-context checks, so do not make this scenario depend on
    // the server completing a separate labelled PART request.
    let alice_nick = alice.snapshot().nick().clone();
    alice
        .send_raw(papo::Line::part_with_reason(
            channel,
            "testing channel context",
        ))
        .await?;
    alice
        .wait_for(|event| {
            event
                .as_parted()
                .filter(|parted| parted.actor.nick().as_str() == alice_nick.as_str())
        })
        .await?;
    bob.wait_for(|event| {
        event
            .as_parted()
            .filter(|parted| parted.actor.nick().as_str() == alice_nick.as_str())
    })
    .await?;
    Ok(())
}

async fn assert_shared_context(
    sender: &Peer,
    recipient: &mut Peer,
    direct: &str,
    channel: &papo::ChannelName,
) -> papo_testing::Result<()> {
    let context_tag_allowed = context_tag_allowed(sender);
    let sent = sender
        .privmsg(direct, "private thread reply")
        .channel_context(channel)
        .await?;
    assert_eq!(
        sent.fallbacks.contains(&Fallback::ContextTagDropped),
        !context_tag_allowed
    );
    let threaded = recipient
        .wait_for(|event| message_text(event, "private thread reply"))
        .await?;
    if context_tag_allowed {
        assert!(
            threaded
                .channel_context()
                .is_some_and(|context| context == channel.as_str())
        );
    } else {
        assert_eq!(threaded.channel_context(), None);
    }
    assert_eq!(threaded.msgid(), Some(confirmed_line(&sent).0));
    Ok(())
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
    assert_eq!(message.kind, MessageKind::Text);
    assert_eq!(
        crate::source_nick(&message.actor),
        Some(alice.snapshot().nick().as_str())
    );
    assert_eq!(
        conversation_name(message.conversation()),
        Some(channel.clone())
    );
    assert_eq!(message.raw_text, "hello from alice");
    assert_eq!(message.text, "hello from alice");
    let (id, _, server_time) = confirmed_line(&sent);
    assert_eq!(message.msgid(), Some(id));
    assert_eq!(message.server_time, server_time);
    assert!(!is_private(&message));
    assert!(sent.fallbacks.is_empty());
    alice
        .expect_none(|event| message_text(event, "hello from alice"))
        .await?;
    bob.expect_none(|event| message_text(event, "hello from alice"))
        .await?;
    Ok(())
}

/// rejects a client line that cannot be relayed within the wire limit
#[scenario(covers = ["base.message.line-too-long"])]
async fn relayed_line_too_long(s: Scenario) -> papo_testing::Result<()> {
    if !matches!(
        papo_test_server::DeploymentProfile::from_env()?,
        papo_test_server::DeploymentProfile::ErgoDefault
            | papo_test_server::DeploymentProfile::ErgoPersistent
    ) {
        return s.skip("the selected server does not reject overlong relayed lines");
    }

    let alice = s.connect("alice").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    let overhead = "PRIVMSG ".len() + channel.len() + " :\r\n".len();
    let text = "x".repeat(papo_proto::MAX_LINE_LEN - overhead);
    let replies = alice
        .request_raw(papo::Line::new("PRIVMSG").with_params([channel, text]))
        .await?;
    let refusal = replies
        .iter()
        .find(|line| line.numeric() == Some(papo::numeric::ERR_INPUTTOOLONG))
        .unwrap_or_else(|| panic!("Ergo omitted numeric 417: {replies:?}"));
    assert!(refusal.params().iter().any(|param| !param.is_empty()));
    Ok(())
}

/// rejects a raw PRIVMSG with an empty trailing parameter
#[scenario(covers = ["base.privmsg.empty-text"])]
async fn empty_privmsg(s: Scenario) -> papo_testing::Result<()> {
    if !matches!(
        papo_test_server::DeploymentProfile::from_env()?,
        papo_test_server::DeploymentProfile::ErgoDefault
            | papo_test_server::DeploymentProfile::ErgoPersistent
    ) {
        return s.skip("the selected server does not report empty PRIVMSG text as numeric 412");
    }

    let alice = s.connect("alice").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    let replies = alice
        .request_raw(papo::Line::new("PRIVMSG").with_params([channel, String::new()]))
        .await?;
    let refusal = replies
        .iter()
        .find(|line| line.numeric() == Some(papo::numeric::ERR_NOTEXTTOSEND))
        .unwrap_or_else(|| panic!("Ergo omitted numeric 412: {replies:?}"));
    assert!(refusal.params().iter().any(|param| !param.is_empty()));
    Ok(())
}

/// peer display text removes terminal controls while retaining raw IRC text
#[scenario(
    requires(caps = ["labeled-response"]),
    covers = ["security.adv-22.display-text-controls"]
)]
async fn display_text_controls(s: Scenario) -> papo_testing::Result<()> {
    let (alice, mut bob, channel) = fixture_pair(&s).await?;
    let raw_text = "\x1b[2Jhello";
    let line = format!("PRIVMSG {channel} :{raw_text}").parse::<papo::Line>()?;

    alice.send_raw(line).await?;
    let message = bob
        .wait_for(|event| {
            event
                .as_message()
                .filter(|message| message.raw_text == raw_text)
        })
        .await?;
    assert_eq!(message.text, "hello");
    assert_eq!(message.raw_text, raw_text);
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
        .wait_for(|event| notice_text(event, "maintenance at midnight"))
        .await?;
    assert_eq!(message.kind, papo::MessageKind::Text);
    assert_eq!(
        crate::source_nick(&message.actor),
        Some(alice.snapshot().nick().as_str())
    );
    assert_eq!(
        conversation_name(message.conversation()),
        Some(channel.clone())
    );
    assert_eq!(message.raw_text, "maintenance at midnight");
    let (id, _, server_time) = confirmed_line(&sent);
    assert_eq!(message.msgid(), Some(id));
    assert_eq!(message.server_time, server_time);
    assert!(message.tags.contains("time"));
    assert!(sent.fallbacks.is_empty());
    alice
        .expect_none(|event| notice_text(event, "maintenance at midnight"))
        .await?;
    bob.expect_none(|event| notice_text(event, "maintenance at midnight"))
        .await?;
    Ok(())
}

/// delivers a by-name private message between peers with no shared channel
#[scenario(covers = ["base.privmsg.user-dm"])]
async fn by_name_direct_message(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let bob_state = bob.snapshot();
    let bob_nick = bob_state.nick().as_str();
    let sent = alice.privmsg(bob_nick, "review is ready").await?;
    assert!(
        matches!(sent.conversation(), papo::Conversation::Private(user) if user.nick() == bob_nick)
    );

    let received = bob
        .wait_for(|event| message_text(event, "review is ready"))
        .await?;
    assert!(is_private(&received));
    assert_eq!(received.actor.nick(), alice.snapshot().nick().as_str());

    let wire = s.rendered_wire_log();
    assert!(
        !wire
            .lines()
            .filter_map(|line| line.split_once(" -> "))
            .any(|(_, line)| {
                papo_proto::LineRef::parse(line).is_ok_and(|line| {
                    line.command_is("WHO") || line.command_is("WHOIS") || line.command_is("MONITOR")
                })
            }),
        "{wire}"
    );
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

    let bob_direct = bob.snapshot().nick().to_string();
    let sent = alice.privmsg(&bob_direct, "private hello").await?;
    let received = bob
        .wait_for(|event| message_text(event, "private hello"))
        .await?;
    assert_eq!(received.kind, MessageKind::Text);
    assert_eq!(
        conversation_name(received.conversation()),
        Some(alice.snapshot().nick().to_string())
    );
    assert_eq!(received.return_address(), alice.snapshot().nick().as_str());
    assert_eq!(received.raw_text, "private hello");
    assert!(is_private(&received));
    let (id, _, server_time) = confirmed_line(&sent);
    assert_delivery_tags(
        received.msgid(),
        received.server_time,
        id,
        server_time,
        papo_test_server::DeploymentProfile::from_env()?,
    );

    let sent = bob
        .notice(alice.snapshot().nick().as_str(), "private notice")
        .await?;
    let received = alice
        .wait_for(|event| notice_text(event, "private notice"))
        .await?;
    assert_eq!(received.kind, papo::MessageKind::Text);
    assert!(
        matches!(received.conversation(), papo::Conversation::Private(user)
        if user.nick() == bob.snapshot().nick().as_str())
    );
    assert_eq!(received.raw_text, "private notice");
    let (id, _, server_time) = confirmed_line(&sent);
    assert_delivery_tags(
        received.msgid(),
        received.server_time,
        id,
        server_time,
        papo_test_server::DeploymentProfile::from_env()?,
    );

    let own = alice.snapshot().nick().clone();
    let sent = alice.privmsg(own, "note to self").await?;
    assert_eq!(
        conversation_name(sent.conversation()),
        Some(alice.snapshot().nick().to_string())
    );
    let (_, sent_text, _) = confirmed_line(&sent);
    assert_eq!(sent_text, "note to self");
    alice
        .expect_none(|event| {
            event
                .as_message()
                .filter(|message| message.raw_text == "note to self")
        })
        .await?;
    assert_eq!(alice.snapshot().nick(), &alice.snapshot().nick().clone());
    assert_eq!(bob.snapshot().nick(), &bob.snapshot().nick().clone());

    let channel = s.channel_name();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let state = alice.snapshot();
    let channel = name_rules(&state).channel(channel);
    assert_shared_context(&alice, &mut bob, &bob_direct, &channel).await?;

    part_for_unverified_context(&mut alice, &mut bob, channel.as_str()).await?;
    assert_unverified_context(&alice, &mut bob, channel.as_str(), "context after part").await?;

    let private_channel = s.channel_name();
    alice.join(&private_channel).await?;
    assert_unverified_context(
        &alice,
        &mut bob,
        &private_channel,
        "context from unshared channel",
    )
    .await?;

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
    if !matches!(
        profile.as_str(),
        "ergo-default" | "ergo-persistent" | "soju-ergo"
    ) {
        return s.skip("the selected profile does not expose account multiclient attachment");
    }
    let (mut desktop, account_key) = s.connect_as_account_with_key("alice").await?;
    let channel = s.channel_name();
    desktop.join(&channel).await?;
    let mut phone = s
        .connect_account_session("alice-phone", account_key)
        .await?;
    phone
        .wait_for(|event| {
            event
                .as_channel_synced()
                .filter(|synced| synced.channel.name().as_str() == channel)
        })
        .await?;
    let phone_state = phone.snapshot();
    let attached = phone_state
        .__channel_local(&channel)
        .expect("another account session inherits joined channels");
    assert!(attached.is_synced());

    let text = "message from the desktop session";
    desktop.privmsg(&channel, text).await?;
    let other = phone
        .wait_for(|event| event.as_message().filter(|message| message.text == text))
        .await?;
    assert_eq!(other.actor_party, papo::Party::Me);
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
                .disable(papo::Feature::EchoMessages),
        )
        .await?;
    assert!(
        phone
            .snapshot()
            .caps()
            .get(&papo::cap::ECHO_MESSAGE)
            .is_none()
    );

    if phone.snapshot().__channel_local(&channel).is_none() {
        phone.join(&channel).await?;
    }
    let text = "message from another logged-in session";
    desktop.privmsg(&channel, text).await?;
    let message = phone
        .wait_for(|event| event.as_message().filter(|message| message.text == text))
        .await?;
    assert_eq!(message.actor_party, papo::Party::Me);
    assert_eq!(
        conversation_name(message.conversation()),
        Some(channel.clone())
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
    let charlie_nick = charlie.snapshot().nick().clone();
    alice
        .wait_for(|event| {
            event
                .as_joined()
                .filter(|joined| joined.actor.nick() == charlie_nick.as_str())
        })
        .await?;

    alice
        .mode(&channel)
        .voice(bob.snapshot().nick().as_str())
        .await?;
    let sent = alice
        .privmsg(format!("+{channel}"), "voiced members only")
        .await?;
    let (sign, received) = restricted_message(
        bob.wait_for(|event| restricted_text(event, "voiced members only"))
            .await?,
    );
    assert_eq!(
        conversation_name(received.conversation()),
        Some(channel.clone())
    );
    assert_eq!(sign, '+');
    assert_eq!(received.raw_text, "voiced members only");
    assert_eq!(received.msgid(), Some(confirmed_line(&sent).0));
    assert!(
        bob.snapshot()
            .__channel_local(&channel)
            .and_then(|members| members.member(bob.snapshot().nick().clone()).unwrap())
            .is_some_and(|member| member
                .prefixes()
                .iter()
                .any(|prefix| prefix.symbol() == '+'))
    );
    charlie
        .expect_none(|event| any_message_text(event, "voiced members only"))
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
    let state = alice.snapshot();
    let channel = name_rules(&state).channel(s.channel_name());
    alice.join(channel.as_str()).await?;
    bob.join(channel.as_str()).await?;
    carol.join(channel.as_str()).await?;
    dave.join(channel.as_str()).await?;

    alice
        .mode(&channel)
        .op(bob.snapshot().nick().as_str())
        .await?;
    alice
        .mode(&channel)
        .voice(carol.snapshot().nick().as_str())
        .await?;
    let sent = carol
        .privmsg(format!("+{channel}"), "voiced members only")
        .await?;
    let received_by_bob = bob
        .wait_for(|event| restricted_text(event, "voiced members only"))
        .await?;
    let received_by_alice = alice
        .wait_for(|event| restricted_text(event, "voiced members only"))
        .await?;
    for restricted in [received_by_bob, received_by_alice] {
        let (sign, message) = restricted_message(restricted);
        assert_eq!(
            conversation_name(message.conversation()),
            Some(channel.to_string())
        );
        assert_eq!(sign, '+');
        assert_eq!(message.raw_text, "voiced members only");
        assert_eq!(message.msgid(), Some(confirmed_line(&sent).0));
    }
    dave.expect_none(|event| any_message_text(event, "voiced members only"))
        .await?;
    carol
        .expect_none(|event| any_message_text(event, "voiced members only"))
        .await?;
    Ok(())
}

/// CTCP actions and automatic private query replies retain their typed forms
#[scenario(
    requires(caps = ["echo-message", "labeled-response"]),
    covers = [
        "base.ctcp.action",
        "base.ctcp.version-query-reply",
        "base.ctcp.ping-roundtrip",
        "security.inj-2.action-segment-framing"
    ]
)]
async fn ctcp(s: Scenario) -> papo_testing::Result<()> {
    let alice_builder = s.configure("alice")?.disable(papo::Feature::Multiline);
    let (mut alice, mut bob, channel) = fixture_pair_with(&s, alice_builder).await?;

    alice.action(&channel, "waves hello").await?;
    let action = bob
        .wait_for(|event| message_text(event, "waves hello"))
        .await?;
    assert_eq!(action.kind, MessageKind::Action);
    assert_eq!(
        conversation_name(action.conversation()),
        Some(channel.clone())
    );
    assert_eq!(action.raw_text, "waves hello");

    let long_action = "waves ".repeat(100);
    alice.action(&channel, &long_action).await?;
    let mut received = String::new();
    while received.len() < long_action.len() {
        let action = bob
            .wait_for(|event| {
                event
                    .as_message()
                    .filter(|message| message.kind == MessageKind::Action)
            })
            .await?;
        received.push_str(&action.raw_text);
    }
    assert_eq!(received, long_action);

    alice
        .send_raw(papo::Line::notice(
            bob.snapshot().nick().as_str(),
            "\u{1}ACTION nods hello\u{1}",
        ))
        .await?;
    let notice = bob
        .wait_for(|event| notice_text(event, "nods hello"))
        .await?;
    assert_eq!(notice.kind, papo::MessageKind::Action);

    alice
        .send_raw(
            papo::Line::new("PRIVMSG")
                .with_params([bob.snapshot().nick().as_str(), "\u{1}VERSION\u{1}"]),
        )
        .await?;
    let _query = bob
        .wait_for(|event| {
            event
                .as_ctcp_query()
                .filter(|ctcp| ctcp.command == "VERSION")
        })
        .await?;
    let reply = alice
        .wait_for(|event| {
            event
                .as_ctcp_reply()
                .filter(|ctcp| ctcp.actor.nick().as_str() == bob.snapshot().nick().as_str())
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
        .send_raw(papo::Line::new("PRIVMSG").with_params([
            carol.snapshot().nick().as_str(),
            "\u{1}PING 1735689600\u{1}",
        ]))
        .await?;
    let query = carol
        .wait_for(|event| event.as_ctcp_query().filter(|ctcp| ctcp.command == "PING"))
        .await?;
    assert_eq!(query.params.as_deref(), Some("1735689600"));
    let reply = alice
        .wait_for(|event| {
            event.as_ctcp_reply().filter(|ctcp| {
                ctcp.actor.nick().as_str() == carol.snapshot().nick().as_str()
                    && ctcp.command == "PING"
            })
        })
        .await?;
    assert_eq!(reply.params.as_deref(), Some("1735689600"));
    Ok(())
}

fn broadcast_mask() -> Option<&'static str> {
    match std::env::var("PAPO_TEST_PROFILE").as_deref() {
        Ok("solanum") => Some("$$*.papo.test"),
        Ok("unrealircd") => Some("$*.papo.test"),
        _ => None,
    }
}

async fn broadcast_operator(s: &Scenario) -> papo_testing::Result<Peer> {
    let operator = s.connect("oper").await?;
    operator
        .oper(
            papo_test_server::OPER_NAME,
            papo::Secret::new(papo_test_server::OPER_PASSWORD),
        )
        .await?;
    Ok(operator)
}

/// delivers a server-mask CTCP query with its original audience
#[scenario(covers = ["base.ctcp.server-mask"])]
async fn broadcast_ctcp_server_mask(s: Scenario) -> papo_testing::Result<()> {
    let Some(mask) = broadcast_mask() else {
        return s.skip("the selected profile does not preserve CTCP server masks");
    };
    let operator = broadcast_operator(&s).await?;
    let mut recipient = s.connect("recipient").await?;
    let nick = operator.snapshot().nick().clone();
    operator
        .send_raw(papo::Line::privmsg(mask, "\u{1}PING 1735689600\u{1}"))
        .await?;
    let broadcast = recipient
        .wait_for(|event| {
            event
                .as_broadcast_ctcp()
                .filter(|ctcp| ctcp.actor.nick() == nick.as_str())
        })
        .await?;
    assert_eq!(broadcast.command, "PING");
    assert_eq!(broadcast.params.as_deref(), Some("1735689600"));
    assert!(matches!(broadcast.to, papo::event::Broadcast::Mask(to) if to.as_str() == mask));
    Ok(())
}

/// delivers a server-mask TAGMSG with its tags and original audience
#[scenario(requires(caps = ["message-tags"]), covers = ["ircv3.message-tags.server-mask"])]
async fn broadcast_tagmsg_server_mask(s: Scenario) -> papo_testing::Result<()> {
    let Some(mask) = broadcast_mask() else {
        return s.skip("the selected profile does not preserve TAGMSG server masks");
    };
    let operator = broadcast_operator(&s).await?;
    let mut recipient = s.connect("recipient").await?;
    let nick = operator.snapshot().nick().clone();
    operator
        .send_raw(papo::Line::tagmsg(mask).with_tag(papo::tags::keys::TYPING, "active"))
        .await?;
    let broadcast = recipient
        .wait_for(|event| {
            event
                .as_broadcast_tag_msg()
                .filter(|tagmsg| tagmsg.actor.nick() == nick.as_str())
        })
        .await?;
    assert_eq!(broadcast.tags.get(papo::tags::keys::TYPING), Some("active"));
    assert!(matches!(broadcast.to, papo::event::Broadcast::Mask(to) if to.as_str() == mask));
    Ok(())
}

/// leaves channel CTCP queries to the application, which can reply privately
#[scenario(covers = [
    "base.ctcp.channel-query-private-reply",
    "security.ctcp.private-target-only"
])]
async fn ctcp_channel_query(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;
    if alice
        .snapshot()
        .__channel_local(&channel)
        .is_some_and(|channel| channel.mode('C').is_some())
    {
        return s.skip("the channel has no-CTCP mode set");
    }
    alice
        .send_raw(papo::Line::new("PRIVMSG").with_params([&channel, "\u{1}VERSION\u{1}"]))
        .await?;
    let query = bob
        .wait_for(|event| {
            event.as_ctcp_query().filter(|ctcp| {
                conversation_name(ctcp.conversation()) == Some(channel.clone())
                    && ctcp.command == "VERSION"
            })
        })
        .await?;
    assert!(matches!(
        query.conversation(),
        papo::Conversation::Channel(_)
    ));
    alice
        .expect_none(|event| {
            event
                .as_ctcp_reply()
                .filter(|ctcp| ctcp.actor.nick().as_str() == bob.snapshot().nick().as_str())
        })
        .await?;
    bob.ctcp_reply(&query, "papo conformance client").await?;
    let reply = alice
        .wait_for(|event| {
            event.as_ctcp_reply().filter(|ctcp| {
                conversation_name(ctcp.conversation()) == Some(bob.snapshot().nick().to_string())
                    && ctcp.actor.nick().as_str() == bob.snapshot().nick().as_str()
            })
        })
        .await?;
    assert!(matches!(
        reply.conversation(),
        papo::Conversation::Private(_)
    ));
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
    let bob_nick = bob.snapshot().nick().clone();
    bob.quit().await?;
    bob.wait_for(Event::as_disconnected).await?;

    alice
        .send_raw(papo::Line::new("PRIVMSG").with_params([bob_nick.as_str(), "\u{1}VERSION\u{1}"]))
        .await?;
    alice
        .expect_none(|event| {
            event
                .as_ctcp_reply()
                .filter(|ctcp| ctcp.actor.nick() == bob_nick.as_str())
        })
        .await?;

    let phone = s.connect_account_session("bob-phone", network).await?;
    let history = phone
        .history(alice.snapshot().nick().as_str())
        .latest()
        .limit(10)
        .await?;
    assert!(history.items.iter().any(|item| matches!(item,
        papo::Event::CtcpQuery(query) if query.command == "VERSION")));
    alice
        .expect_none(|event| {
            event
                .as_ctcp_reply()
                .filter(|ctcp| ctcp.actor.nick() == bob_nick.as_str())
        })
        .await?;
    Ok(())
}

/// blocks channel CTCP traffic when the server's no-CTCP mode is set
#[scenario(covers = ["base.ctcp.no-ctcp-mode"])]
async fn no_ctcp_mode(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, bob, channel) = fixture_pair(&s).await?;
    let supports_no_ctcp = alice
        .snapshot()
        .isupport()
        .chanmodes()
        .is_some_and(|groups| {
            groups.list.contains('C')
                || groups.always.contains('C')
                || groups.when_set.contains('C')
                || groups.never.contains('C')
        });
    if !supports_no_ctcp {
        return s.skip("CHANMODES does not advertise no-CTCP mode C");
    }
    alice.mode(&channel).add('C').await?;
    bob.send_raw(papo::Line::new("PRIVMSG").with_params([&channel, "\u{1}VERSION\u{1}"]))
        .await?;
    alice
        .expect_none(|event| {
            event.as_ctcp_query().filter(|ctcp| {
                conversation_name(ctcp.conversation()) == Some(channel.clone())
                    && ctcp.command == "VERSION"
            })
        })
        .await?;
    Ok(())
}

fn assert_confirmed_as_seen(
    outcome: &papo::ReactOutcome,
    seen: &papo::event::Reacted,
    target: &str,
) {
    let confirmed = match &outcome.confirmation {
        papo::Confirmation::Confirmed { msgid, .. } => Some(msgid),
        papo::Confirmation::Unconfirmed {
            reason: papo::UnconfirmedReason::NoMessageId,
            ..
        } => None,
        other => panic!("echo-message and labeled-response confirm a reaction: {other:?}"),
    };
    assert_eq!(confirmed, seen.msgid());
    assert_eq!(outcome.status, None);
    assert_eq!(
        conversation_name(&outcome.conversation),
        Some(target.to_owned())
    );
}

async fn channel_original(
    alice: &Peer,
    bob: &mut Peer,
    channel: &str,
) -> papo_testing::Result<(papo::Message, papo::MessageId)> {
    alice.privmsg(channel, "ship release 2.19").await?;
    let original = bob
        .wait_for(|event| message_text(event, "ship release 2.19"))
        .await?;
    let original_id = original.msgid().expect("message has an identifier").clone();
    Ok((original, original_id))
}

/// a reply retains the referenced message identifier
#[scenario(
    requires(caps = ["echo-message", "labeled-response", "message-tags"]),
    covers = ["tag.reply"]
)]
async fn replies(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;
    let (original, original_id) = channel_original(&alice, &mut bob, &channel).await?;

    let sent = bob.reply(&original, "acknowledged").await?;
    if sent.fallbacks.contains(&Fallback::ReplyTagDropped) {
        return s.skip("server denies both supported reply-tag aliases");
    }
    let reply = alice
        .wait_for(|event| message_text(event, "acknowledged"))
        .await?;
    assert_eq!(reply.raw_text, "acknowledged");
    assert_eq!(reply.in_reply_to.as_ref(), Some(&original_id));
    assert_eq!(conversation_name(reply.conversation()), Some(channel));
    Ok(())
}

/// reaction changes retain the referenced message identifier
#[scenario(
    requires(caps = ["echo-message", "labeled-response", "message-tags"]),
    covers = ["tag.react"]
)]
async fn reactions(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;
    let (original, original_id) = channel_original(&alice, &mut bob, &channel).await?;

    let reacted = reaction_or_skip(bob.react(&original, "👍").await)?;
    let added = alice.wait_for(Event::as_reacted).await?;
    assert_confirmed_as_seen(&reacted, &added, &channel);
    assert_eq!(
        conversation_name(added.conversation()),
        Some(channel.clone())
    );
    assert_eq!(added.reacts_to, original_id);
    assert_eq!(added.reaction, "👍");
    assert_eq!(added.action, papo::event::ReactionAction::Added);
    assert_eq!(
        crate::source_nick(&added.actor),
        Some(bob.snapshot().nick().as_str())
    );

    let unreacted = bob.unreact(&original, "👍").await?;
    let removed = alice.wait_for(Event::as_reacted).await?;
    assert_confirmed_as_seen(&unreacted, &removed, &channel);
    assert_eq!(
        conversation_name(removed.conversation()),
        Some(channel.clone())
    );
    assert_eq!(removed.reacts_to, original_id);
    assert_eq!(removed.reaction, "👍");
    assert_eq!(removed.action, papo::event::ReactionAction::Removed);
    Ok(())
}

/// a reaction whose tag the server's `CLIENTTAGDENY` blocks is refused before it is sent
#[scenario(
    requires(caps = ["message-tags"], isupport = ["CLIENTTAGDENY"]),
    covers = ["ircv3.message-tags.clienttagdeny"]
)]
async fn client_tag_deny(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, mut bob, channel) = fixture_pair(&s).await?;
    if !bob
        .snapshot()
        .isupport()
        .clienttagdeny()
        .denies("+draft/react")
    {
        return s.skip("CLIENTTAGDENY allows the reaction tag");
    }
    let (original, _) = channel_original(&alice, &mut bob, &channel).await?;

    let refused = bob.react(&original, "👍").await;
    assert!(
        matches!(
            &refused,
            Err(Error::Unsupported(unsupported))
                if unsupported.feature == papo::Feature::Reactions
                    && unsupported.reason == papo::UnsupportedReason::TagDenied
        ),
        "a denied reaction tag was not refused: {refused:?}"
    );
    assert!(
        !s.rendered_wire_log()
            .lines()
            .any(|line| line.contains(" -> ") && line.contains(" TAGMSG ")),
        "a refused reaction reached the wire"
    );
    alice.expect_none(Event::as_reacted).await?;
    Ok(())
}

/// a private reaction to an away user is confirmed by its echo
#[scenario(
    requires(caps = ["echo-message", "labeled-response", "message-tags"]),
    covers = ["tag.react.private-to-away-user"]
)]
async fn private_reaction_to_away_user(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let bob_nick = bob.snapshot().nick().to_string();
    bob.privmsg(alice.snapshot().nick().as_str(), "the build is green again")
        .await?;
    let original = alice
        .wait_for(|event| message_text(event, "the build is green again"))
        .await?;
    bob.set_away("out for lunch until 14:00").await?;
    assert!(matches!(
        bob.snapshot().away(),
        papo::AwayStatus::Away { .. }
    ));

    let reacted = reaction_or_skip(alice.react(&original, "👍").await)?;
    let seen = bob.wait_for(Event::as_reacted).await?;
    assert!(
        matches!(reacted.confirmation, papo::Confirmation::Confirmed { .. }),
        "the echo confirms a reaction to an away user: {reacted:?}"
    );
    assert_confirmed_as_seen(&reacted, &seen, &bob_nick);
    assert_eq!(
        seen.reacts_to,
        *original.msgid().expect("message has an identifier")
    );
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
    let alice_nick = alice.snapshot().nick().clone();
    let bob_nick = bob.snapshot().nick().clone();

    alice.typing(bob_nick.as_str(), Typing::Active).await?;
    let typing = bob.wait_for(Event::as_typing_changed).await?;

    assert_eq!(crate::source_nick(&typing.actor), Some(alice_nick.as_str()));
    assert_eq!(
        conversation_name(typing.conversation()),
        Some(alice_nick.to_string())
    );
    assert_eq!(typing.state, Typing::Active);

    alice.typing(bob_nick.as_str(), Typing::Active).await?;
    bob.expect_none(Event::as_typing_changed).await?;
    alice.typing(bob_nick.as_str(), Typing::Paused).await?;
    assert_eq!(
        bob.wait_for(Event::as_typing_changed).await?.state,
        Typing::Paused
    );
    alice.typing(bob_nick.as_str(), Typing::Done).await?;
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
    let state = alice.snapshot();
    let channel = name_rules(&state).channel(s.channel_name());
    alice.join(channel.as_str()).await?;
    bob.join(channel.as_str()).await?;
    carol.join(channel.as_str()).await?;
    dave.join(channel.as_str()).await?;
    alice
        .mode(&channel)
        .op(bob.snapshot().nick().as_str())
        .await?;

    let target = format!("@{channel}");
    alice.typing(&target, Typing::Active).await?;
    let restricted = bob
        .wait_for(|event| restricted_line(event, papo::RestrictedLine::as_typing_changed))
        .await?;
    assert_eq!(restricted.audience.symbol(), '@');
    let typing = restricted.line.as_typing_changed().expect("typing line");
    assert_eq!(
        crate::source_nick(&typing.actor),
        Some(alice.snapshot().nick().as_str())
    );
    assert_eq!(
        conversation_name(typing.conversation()),
        Some(channel.to_string())
    );
    assert_eq!(typing.state, Typing::Active);
    for peer in [&mut carol, &mut dave] {
        peer.expect_none(|event| {
            restricted_line(event, papo::RestrictedLine::as_typing_changed)
                .map(|_| event)
                .or_else(|| event.as_typing_changed().map(|_| event))
        })
        .await?;
    }
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
    assert_eq!(received.kind, MessageKind::Text);
    assert_eq!(
        conversation_name(received.conversation()),
        Some(channel.clone())
    );
    assert_eq!(received.raw_text, text);
    assert_eq!(received.text, text);
    let (id, _, server_time) = confirmed_line(&sent);
    assert_eq!(received.msgid(), Some(id));
    assert_eq!(received.server_time, server_time);
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
    assert_eq!(received.msgid(), Some(confirmed_line(&sent).0));
    Ok(())
}

/// Whether the server answered one of this scenario's labeled requests with a bare `ACK`.
fn received_labeled_ack(s: &Scenario) -> bool {
    s.rendered_wire_log()
        .lines()
        .any(|line| line.contains(" <- @") && line.contains("label=") && line.ends_with(" ACK"))
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
        "draft.message-redaction.errors",
        "ircv3.labeled-response.ack-no-reply"
    ]
)]
async fn draft_message_redaction(s: Scenario) -> papo_testing::Result<()> {
    let (alice, mut bob, channel) = fixture_pair(&s).await?;
    let sent = alice.privmsg(&channel, "message to\nredact").await?;
    let received = bob
        .wait_for(|event| message_text(event, "message to\nredact"))
        .await?;
    let id = received.msgid().expect("message has an identifier").clone();
    assert_eq!(&id, confirmed_line(&sent).0);

    let outcome = alice.redact(&sent, "superseded").await?;
    assert!(
        received_labeled_ack(&s),
        "the redaction was not acknowledged with a labeled ACK"
    );
    let redacted = bob.wait_for(Event::as_redacted).await?;
    let (redacts, echo, echo_time) = confirmed_redaction(&outcome);
    assert_eq!(redacts, &id);
    assert_eq!(redacted.msgid(), Some(echo));
    assert_eq!(redacted.server_time, echo_time);
    assert_eq!(
        conversation_name(redacted.conversation()),
        Some(channel.clone())
    );
    assert_eq!(redacted.redacts, id);
    assert_eq!(redacted.reason.as_deref(), Some("superseded"));
    assert_eq!(
        crate::source_nick(&redacted.actor),
        Some(alice.snapshot().nick().as_str())
    );
    bob.expect_none(Event::as_redacted).await?;

    let Error::Server(refusal) = alice
        .redact(&sent, "already redacted")
        .await
        .expect_err("a message redacted once is unknown to the server")
    else {
        panic!("an unknown message identifier did not return a server refusal")
    };
    assert_eq!(refusal.refusal(), Refusal::UnknownMessage);
    assert_eq!(refusal.code(), "UNKNOWN_MSGID");
    bob.expect_none(Event::as_redacted).await?;
    Ok(())
}

/// a redaction from a session without echo-message is confirmed by the echo Ergo still sends it
#[scenario(
    requires(caps = ["draft/message-redaction", "labeled-response"]),
    covers = ["draft.message-redaction.confirm-without-echo-message"]
)]
async fn redaction_without_echo_message(s: Scenario) -> papo_testing::Result<()> {
    let (mut alice, mut bob, channel) = fixture_pair_with(
        &s,
        s.configure("alice")?.disable(papo::Feature::EchoMessages),
    )
    .await?;
    bob.privmsg(&channel, "deploy finished at 14:05").await?;
    let received = alice
        .wait_for(|event| message_text(event, "deploy finished at 14:05"))
        .await?;

    let outcome = alice.redact(&received, "wrong time").await?;
    let redacted = bob.wait_for(Event::as_redacted).await?;
    let (redacts, echo, echo_time) = confirmed_redaction(&outcome);
    assert_eq!(Some(redacts), received.msgid());
    assert_eq!(redacted.msgid(), Some(echo));
    assert_eq!(redacted.server_time, echo_time);
    Ok(())
}

/// a private history message keeps the channel context its line names, and replies keep it; the
/// sender is logged in because only a history line's account tag identifies who to reply to
#[scenario(
    requires(caps = ["account-tag", "batch", "draft/chathistory", "message-tags"]),
    covers = ["tag.channel-context"]
)]
async fn history_private_channel_context(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let bob = s.connect_as_account("bob").await?;
    let (mut alice, mut bob, channel) = joined_pair(&s, alice, bob).await?;
    if !context_tag_allowed(&bob) {
        return s.skip("the server denies the channel-context client tag");
    }
    let text = "can you check the release notes before the vote?";
    bob.privmsg(alice.snapshot().nick().as_str(), text)
        .channel_context(channel.as_str())
        .await?;
    let live = alice.wait_for(|event| message_text(event, text)).await?;
    assert!(
        live.channel_context()
            .is_some_and(|context| context == channel.as_str())
    );

    let page = alice
        .history(bob.snapshot().nick().as_str())
        .latest()
        .limit(10)
        .await?;
    let Some(historical) = page
        .chat_messages()
        .find(|message| message.raw_text == text)
    else {
        return s.skip("the server keeps no direct-message history for these clients");
    };
    let stored = historical
        .tags
        .get("+channel-context")
        .or_else(|| historical.tags.get("+draft/channel-context"));
    if stored.is_none() {
        assert_eq!(historical.channel_context(), None);
        return s.skip("the server does not store client-only tags in history");
    }
    assert_eq!(historical.channel_context(), live.channel_context());

    let answer = "notes look good, ship it";
    alice.reply(historical, answer).await?;
    let reply = bob.wait_for(|event| message_text(event, answer)).await?;
    assert!(
        reply
            .channel_context()
            .is_some_and(|context| context == channel.as_str())
    );
    assert_eq!(reply.in_reply_to.as_ref(), historical.msgid());
    Ok(())
}

/// a service's reply to our labelled message reaches the application
#[scenario(covers = ["regression.labeled-response.service-reply-in-send-batch"])]
async fn service_reply_to_a_labelled_message(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    alice.privmsg("NickServ", "HELP").await?;
    let reply = alice
        .wait_for(|event| {
            event.as_notice().filter(|notice| {
                notice
                    .actor
                    .nick()
                    .as_str()
                    .eq_ignore_ascii_case("NickServ")
            })
        })
        .await?;
    assert!(conversation_is_private(reply.conversation()));
    assert!(!reply.text.is_empty());
    Ok(())
}
