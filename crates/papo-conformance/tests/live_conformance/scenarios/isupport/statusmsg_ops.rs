use papo::{Event, ModeAction, ModeArg, Typing};
use papo_conformance_macros::scenario;

use crate::scenarios::support::{
    any_message_text, confirmed_line, join_seen_by, message_text, reaction_or_skip,
    restricted_line, restricted_message, restricted_text, wait_channel_mode, wait_for_channel,
};
use crate::{Peer, Scenario};

struct StatusmsgPeers {
    alice: Peer,
    bob: Peer,
    carol: Peer,
    dave: Peer,
    channel: String,
}

async fn statusmsg_peers(s: &Scenario) -> papo_testing::Result<StatusmsgPeers> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let mut carol = s.connect("carol").await?;
    let mut dave = s.connect("dave").await?;
    let channel = s.channel_name();
    let alice_nick = alice.snapshot().nick().clone();
    let bob_nick = bob.snapshot().nick().clone();
    let carol_nick = carol.snapshot().nick().clone();

    alice.join(&channel).await?;
    wait_for_channel(&mut alice, &channel).await?;
    for peer in [&mut bob, &mut carol, &mut dave] {
        join_seen_by(peer, &mut alice, &channel).await?;
        wait_for_channel(peer, &channel).await?;
    }

    assert!(
        alice
            .snapshot()
            .isupport()
            .statusmsg()
            .is_some_and(|tokens| tokens.contains('@'))
    );

    alice.set_modes(&channel).op(bob_nick.as_str()).await?;
    for (name, peer) in [
        ("alice", &mut alice),
        ("bob", &mut bob),
        ("carol", &mut carol),
        ("dave", &mut dave),
    ] {
        let changed = wait_channel_mode(peer, &channel, 'o').await?;
        assert_eq!(
            crate::source_nick(&changed.actor),
            Some(alice_nick.as_str())
        );
        let [change] = changed.changes.as_slice() else {
            panic!("operator grant should contain one change: {changed:?}")
        };
        assert_eq!(change.action, ModeAction::Add);
        assert!(
            matches!(change.arg.clone(), Some(ModeArg::Member { user, .. }) if user.nick() == bob_nick.as_str())
        );

        let state = peer.snapshot();
        let member = state
            .channel(&channel)?
            .expect("channel is in state")
            .member(bob_nick.as_str())
            .unwrap()
            .unwrap();
        assert_eq!(
            member.prefixes(),
            &[papo::Prefix::__new('o', '@')],
            "{name} has Bob's operator rank"
        );
    }

    alice.set_modes(&channel).voice(carol_nick.as_str()).await?;
    for (name, peer) in [
        ("alice", &mut alice),
        ("bob", &mut bob),
        ("carol", &mut carol),
        ("dave", &mut dave),
    ] {
        let changed = wait_channel_mode(peer, &channel, 'v').await?;
        assert_eq!(
            crate::source_nick(&changed.actor),
            Some(alice_nick.as_str())
        );
        let [change] = changed.changes.as_slice() else {
            panic!("voice grant should contain one change: {changed:?}")
        };
        assert_eq!(change.action, ModeAction::Add);
        assert!(
            matches!(change.arg.clone(), Some(ModeArg::Member { user, .. }) if user.nick() == carol_nick.as_str())
        );

        let state = peer.snapshot();
        let member = state
            .channel(&channel)?
            .expect("channel is in state")
            .member(carol_nick.as_str())
            .unwrap()
            .unwrap();
        assert_eq!(
            member.prefixes(),
            &[papo::Prefix::__new('v', '+')],
            "{name} has Carol's voice rank"
        );
    }

    Ok(StatusmsgPeers {
        alice,
        bob,
        carol,
        dave,
        channel,
    })
}

/// an @-prefixed channel message reaches only channel operators
#[scenario(
    requires(caps = ["echo-message", "labeled-response"], isupport = ["STATUSMSG"]),
    covers = [
        "base.privmsg.statusmsg",
        "base.isupport.statusmsg-token",
        "base.privmsg.statusmsg-ops"
    ]
)]
async fn statusmsg_ops(s: Scenario) -> papo_testing::Result<()> {
    let StatusmsgPeers {
        alice,
        mut bob,
        mut carol,
        mut dave,
        channel,
    } = statusmsg_peers(&s).await?;
    let alice_nick = alice.snapshot().nick().clone();

    let sent = alice
        .privmsg(format!("@{channel}"), "operators only")
        .await?;
    let (sign, received) = restricted_message(
        bob.wait_for(|event| restricted_text(event, "operators only"))
            .await?,
    );
    assert_eq!(
        crate::scenarios::support::conversation_name(received.conversation()),
        Some(channel.clone())
    );
    assert_eq!(sign, '@');
    assert_eq!(received.raw_text, "operators only");
    let (id, text, _) = confirmed_line(&sent);
    assert_eq!(text, "operators only");
    assert_eq!(received.msgid(), Some(id));
    assert_eq!(
        crate::source_nick(&received.actor),
        Some(alice_nick.as_str())
    );

    for peer in [&mut carol, &mut dave] {
        peer.expect_none(|event| any_message_text(event, "operators only"))
            .await?;
    }
    Ok(())
}

/// Sends an operators-only question and asserts that only the operator `bob` receives it.
async fn ops_question(peers: &mut StatusmsgPeers) -> papo_testing::Result<papo::event::Message> {
    let channel = &peers.channel;
    peers
        .alice
        .privmsg(format!("@{channel}"), "operators-only question")
        .await?;
    let (sign, question) = restricted_message(
        peers
            .bob
            .wait_for(|event| restricted_text(event, "operators-only question"))
            .await?,
    );
    assert_eq!(
        crate::scenarios::support::conversation_name(question.conversation()),
        Some(channel.clone())
    );
    assert_eq!(sign, '@');
    for peer in [&mut peers.carol, &mut peers.dave] {
        peer.expect_none(|event| any_message_text(event, "operators-only question"))
            .await?;
    }
    Ok(question)
}

/// Asserts that neither non-operator receives the line, as an operators-only or a plain line.
async fn unseen_by_non_operators<T>(
    peers: &mut StatusmsgPeers,
    restricted: fn(&papo::event::RestrictedLine) -> Option<&T>,
    plain: fn(&Event) -> Option<&T>,
) -> papo_testing::Result<()> {
    for peer in [&mut peers.carol, &mut peers.dave] {
        peer.expect_none(|event| {
            restricted_line(event, restricted)
                .map(|_| event)
                .or_else(|| plain(event).map(|_| event))
        })
        .await?;
    }
    Ok(())
}

/// preserves an operators-only audience when replying to a status message
#[scenario(
    requires(caps = ["echo-message", "labeled-response"], isupport = ["STATUSMSG"]),
    covers = [
        "base.privmsg.statusmsg-reply-ops",
        "security.routing.status-prefix-channel"
    ]
)]
async fn statusmsg_reply_ops(s: Scenario) -> papo_testing::Result<()> {
    let mut peers = statusmsg_peers(&s).await?;
    let question = ops_question(&mut peers).await?;

    peers.bob.reply(&question, "operators-only answer").await?;
    let (sign, answer) = restricted_message(
        peers
            .alice
            .wait_for(|event| restricted_text(event, "operators-only answer"))
            .await?,
    );
    assert_eq!(
        crate::scenarios::support::conversation_name(answer.conversation()),
        Some(peers.channel.clone())
    );
    assert_eq!(sign, '@');
    for peer in [&mut peers.carol, &mut peers.dave] {
        peer.expect_none(|event| any_message_text(event, "operators-only answer"))
            .await?;
    }
    Ok(())
}

/// preserves an operators-only audience when reacting to a status message
#[scenario(
    requires(
        caps = ["echo-message", "labeled-response", "message-tags"],
        isupport = ["STATUSMSG"]
    ),
    covers = ["security.routing.status-prefix-reaction"]
)]
async fn statusmsg_reaction_ops(s: Scenario) -> papo_testing::Result<()> {
    let mut peers = statusmsg_peers(&s).await?;
    let question = ops_question(&mut peers).await?;
    reaction_or_skip(peers.bob.react(&question, "🛡️").await)?;
    expect_ops_reaction(&mut peers, &question).await
}

/// preserves an operators-only audience when reacting to a saved message reference
#[scenario(
    requires(
        caps = ["echo-message", "labeled-response", "message-tags"],
        isupport = ["STATUSMSG"]
    ),
    covers = ["security.statusmsg.saved-reference"]
)]
async fn statusmsg_saved_reaction_ops(s: Scenario) -> papo_testing::Result<()> {
    let mut peers = statusmsg_peers(&s).await?;
    let question = ops_question(&mut peers).await?;
    let saved = question.return_address().to_saved();
    let restored = peers.bob.saved_message(&saved)?;
    reaction_or_skip(peers.bob.react(restored, "🛡️").await)?;
    expect_ops_reaction(&mut peers, &question).await
}

async fn expect_ops_reaction(
    peers: &mut StatusmsgPeers,
    question: &papo::event::Message,
) -> papo_testing::Result<()> {
    let question_id = question
        .msgid()
        .expect("operators-only message has an identifier");
    let reaction = peers
        .alice
        .wait_for(|event| {
            restricted_line(event, |line| {
                line.as_reacted().filter(|reaction| {
                    &reaction.reacts_to == question_id && reaction.reaction == "🛡️"
                })
            })
        })
        .await?;
    assert_eq!(reaction.audience.symbol(), '@');
    unseen_by_non_operators(
        peers,
        papo::event::RestrictedLine::as_reacted,
        Event::as_reacted,
    )
    .await
}

/// preserves an operators-only audience when typing in answer to a status message
#[scenario(
    requires(caps = ["message-tags"], isupport = ["STATUSMSG"]),
    covers = ["security.routing.status-prefix-typing"]
)]
async fn statusmsg_typing_ops(s: Scenario) -> papo_testing::Result<()> {
    let mut peers = statusmsg_peers(&s).await?;
    let question = ops_question(&mut peers).await?;

    let typed = peers
        .bob
        .set_typing(question.return_address(), Typing::Active)
        .await;
    if matches!(
        &typed,
        Err(papo::Error::Unsupported(unsupported))
            if unsupported.reason == papo::UnsupportedReason::TagDenied
    ) {
        return s.skip("server denies the typing tag");
    }
    typed?;
    let typing = peers
        .alice
        .wait_for(|event| restricted_line(event, papo::event::RestrictedLine::as_typing_changed))
        .await?;
    assert_eq!(typing.audience.symbol(), '@');
    assert_eq!(
        typing.line.as_typing_changed().map(|typing| typing.state),
        Some(Typing::Active)
    );
    unseen_by_non_operators(
        &mut peers,
        papo::event::RestrictedLine::as_typing_changed,
        Event::as_typing_changed,
    )
    .await
}

/// posts to everyone from the window of an operators-only line and refuses stacked prefixes
#[scenario(
    requires(caps = ["echo-message", "labeled-response"], isupport = ["STATUSMSG"]),
    covers = [
        "security.statusmsg.window-reaches-everyone",
        "security.statusmsg.stacked-status-refused"
    ]
)]
async fn statusmsg_window_reaches_everyone(s: Scenario) -> papo_testing::Result<()> {
    let StatusmsgPeers {
        mut alice,
        mut bob,
        mut carol,
        mut dave,
        channel,
    } = statusmsg_peers(&s).await?;

    alice
        .privmsg(format!("@{channel}"), "operators-only question")
        .await?;
    let (_, question) = restricted_message(
        bob.wait_for(|event| restricted_text(event, "operators-only question"))
            .await?,
    );
    let papo::Conversation::Channel(window) = question.conversation() else {
        panic!("a channel line files under its channel")
    };
    bob.privmsg(window, "posted from the window").await?;
    for peer in [&mut alice, &mut carol, &mut dave] {
        peer.wait_for(|event| message_text(event, "posted from the window"))
            .await?;
    }

    let stacked = bob
        .privmsg(format!("+@{channel}"), "stacked audience")
        .await;
    assert!(
        matches!(
            stacked,
            Err(papo::Error::InvalidInput(papo::InvalidInput::InvalidName {
                reason: papo::InvalidIdentifier::InvalidPrefix('@'),
                ..
            }))
        ),
        "stacked prefixes were not refused: {stacked:?}"
    );
    for peer in [&mut alice, &mut carol, &mut dave] {
        peer.expect_none(|event| any_message_text(event, "stacked audience"))
            .await?;
    }
    Ok(())
}

/// an application that never matches the restricted arm never answers an operators-only line
#[scenario(
    requires(isupport = ["STATUSMSG"]),
    covers = ["security.statusmsg.unhandled-arm-never-answers"]
)]
async fn statusmsg_unhandled_arm_never_answers(s: Scenario) -> papo_testing::Result<()> {
    let StatusmsgPeers {
        alice,
        mut bob,
        carol: _,
        mut dave,
        channel,
    } = statusmsg_peers(&s).await?;

    alice
        .privmsg(format!("@{channel}"), "!deploy key rotated?")
        .await?;
    alice.privmsg(&channel, "!ping").await?;
    let command = bob
        .wait_for(|event| {
            event
                .as_message()
                .filter(|message| message.text.starts_with('!'))
        })
        .await?;
    assert_eq!(command.text, "!ping");
    bob.privmsg(&channel, format!("handled {}", command.text))
        .await?;

    dave.wait_for(|event| message_text(event, "handled !ping"))
        .await?;
    dave.expect_none(|event| any_message_text(event, "handled !deploy key rotated?"))
        .await?;
    Ok(())
}
