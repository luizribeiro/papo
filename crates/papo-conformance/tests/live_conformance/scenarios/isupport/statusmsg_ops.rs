use papo::{ModeAction, ModeArg, Typing};
use papo_conformance_macros::scenario;

use crate::scenarios::support::{
    confirmed_line, join_seen_by, message_text, wait_channel_mode, wait_for_channel,
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
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    let carol_nick = carol.nick();

    alice.join(&channel).await?;
    wait_for_channel(&mut alice, &channel).await?;
    for peer in [&mut bob, &mut carol, &mut dave] {
        join_seen_by(peer, &mut alice, &channel).await?;
        wait_for_channel(peer, &channel).await?;
    }

    assert!(
        alice
            .state()
            .isupport()
            .statusmsg()
            .is_some_and(|tokens| tokens.contains('@'))
    );

    let bob_direct = alice.direct(bob_nick.as_ref()).await?;
    let carol_direct = alice.direct(carol_nick.as_ref()).await?;
    alice.mode(&channel).op(&bob_direct).await?;
    for (name, peer) in [
        ("alice", &mut alice),
        ("bob", &mut bob),
        ("carol", &mut carol),
        ("dave", &mut dave),
    ] {
        let changed = wait_channel_mode(peer, &channel, 'o').await?;
        assert_eq!(
            crate::source_nick(&changed.actor),
            Some(alice_nick.as_ref())
        );
        let [change] = changed.changes.as_slice() else {
            panic!("operator grant should contain one change: {changed:?}")
        };
        assert_eq!(change.action, ModeAction::Add);
        assert!(
            matches!(change.arg.clone(), Some(ModeArg::Member { nick, .. }) if nick == bob_nick.as_ref())
        );

        let state = peer.state();
        let member = state
            .channel(&channel)
            .expect("channel is in state")
            .member(bob_nick.as_ref())
            .unwrap();
        assert_eq!(
            member.prefixes(),
            &[papo::Prefix::__new('o', '@')],
            "{name} has Bob's operator rank"
        );
    }

    alice.mode(&channel).voice(&carol_direct).await?;
    for (name, peer) in [
        ("alice", &mut alice),
        ("bob", &mut bob),
        ("carol", &mut carol),
        ("dave", &mut dave),
    ] {
        let changed = wait_channel_mode(peer, &channel, 'v').await?;
        assert_eq!(
            crate::source_nick(&changed.actor),
            Some(alice_nick.as_ref())
        );
        let [change] = changed.changes.as_slice() else {
            panic!("voice grant should contain one change: {changed:?}")
        };
        assert_eq!(change.action, ModeAction::Add);
        assert!(
            matches!(change.arg.clone(), Some(ModeArg::Member { nick, .. }) if nick == carol_nick.as_ref())
        );

        let state = peer.state();
        let member = state
            .channel(&channel)
            .expect("channel is in state")
            .member(carol_nick.as_ref())
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
    let alice_nick = alice.nick();

    let sent = alice
        .privmsg(format!("@{channel}"), "operators only")
        .await?;
    let received = bob
        .wait_for(|event| message_text(event, "operators only"))
        .await?;
    assert_eq!(
        crate::scenarios::support::conversation_name(&received.conversation),
        Some(channel.as_str())
    );
    assert_eq!(received.status.map(papo::Prefix::symbol), Some('@'));
    assert_eq!(received.raw_text, "operators only");
    let (id, text, _) = confirmed_line(&sent);
    assert_eq!(text, "operators only");
    assert_eq!(received.msgid.as_ref(), Some(id));
    assert_eq!(
        crate::source_nick(&received.actor),
        Some(alice_nick.as_ref())
    );

    for peer in [&mut carol, &mut dave] {
        peer.expect_none(|event| message_text(event, "operators only"))
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
    let question = bob
        .wait_for(|event| message_text(event, "operators-only question"))
        .await?;
    assert_eq!(
        crate::scenarios::support::conversation_name(&question.conversation),
        Some(channel.as_str())
    );
    assert_eq!(question.status.map(papo::Prefix::symbol), Some('@'));
    for peer in [&mut carol, &mut dave] {
        peer.expect_none(|event| message_text(event, "operators-only question"))
            .await?;
    }

    bob.reply(&question, "operators-only answer").await?;
    let answer = alice
        .wait_for(|event| message_text(event, "operators-only answer"))
        .await?;
    assert_eq!(
        crate::scenarios::support::conversation_name(&answer.conversation),
        Some(channel.as_str())
    );
    assert_eq!(answer.status.map(papo::Prefix::symbol), Some('@'));
    for peer in [&mut carol, &mut dave] {
        peer.expect_none(|event| message_text(event, "operators-only answer"))
            .await?;
    }

    let profile = std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into());
    if !matches!(profile.as_str(), "ergo-default" | "ergo-persistent") {
        return Ok(());
    }

    let question_id = question
        .msgid
        .as_ref()
        .expect("operators-only message has an identifier")
        .clone();
    bob.react(&question, "🛡️").await?;
    let reaction = alice
        .wait_for(|event| {
            event
                .as_reacted()
                .filter(|reaction| reaction.reacts_to == question_id && reaction.reaction == "🛡️")
        })
        .await?;
    assert_eq!(reaction.status.map(papo::Prefix::symbol), Some('@'));
    for peer in [&mut carol, &mut dave] {
        peer.expect_none(|event| {
            event
                .as_reacted()
                .filter(|reaction| reaction.reaction == "🛡️")
        })
        .await?;
    }

    bob.typing(question.return_address(), Typing::Active)
        .await?;
    let typing = alice.wait_for(papo::Event::as_typing_changed).await?;
    assert_eq!(typing.status.map(papo::Prefix::symbol), Some('@'));
    assert_eq!(typing.state, Typing::Active);
    for peer in [&mut carol, &mut dave] {
        peer.expect_none(papo::Event::as_typing_changed).await?;
    }

    bob.privmsg(&channel, "bare-channel threaded answer")
        .reply_to(&question_id)
        .await?;
    for peer in [&mut alice, &mut carol, &mut dave] {
        let leaked = peer
            .wait_for(|event| message_text(event, "bare-channel threaded answer"))
            .await?;
        assert_eq!(leaked.status, None);
        assert_eq!(leaked.in_reply_to.as_ref(), Some(&question_id));
    }

    let bare_channel = bob
        .state()
        .name_rules()
        .expect("server advertised CASEMAPPING")
        .channel(&channel);
    bob.react((&bare_channel, &question_id), "📣").await?;
    for peer in [&mut alice, &mut carol, &mut dave] {
        let leaked = peer
            .wait_for(|event| {
                event.as_reacted().filter(|reaction| {
                    reaction.reacts_to == question_id && reaction.reaction == "📣"
                })
            })
            .await?;
        assert_eq!(leaked.status, None);
    }
    Ok(())
}
