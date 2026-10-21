use papo::{ModeAction, ModeArg, Origin};
use papo_conformance_macros::scenario;
use papo_testing::Peer;

use crate::Scenario;
use crate::scenarios::support::{join_seen_by, message_text, wait_channel_mode};

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
    for peer in [&bob, &carol, &dave] {
        join_seen_by(peer, &mut alice, &channel).await?;
    }

    assert!(
        alice
            .state()
            .isupport()
            .statusmsg()
            .is_some_and(|tokens| tokens.contains('@'))
    );

    alice
        .mode(&channel)
        .op(bob_nick.clone())
        .voice(carol_nick.clone())
        .await?;
    for peer in [&mut alice, &mut bob, &mut carol, &mut dave] {
        let changed = wait_channel_mode(peer, &channel, 'v').await?;
        assert_eq!(
            crate::source_nick(&changed.actor),
            Some(alice_nick.as_ref())
        );
        assert_eq!(changed.changes.len(), 2);
        for (letter, nick) in [('o', &bob_nick), ('v', &carol_nick)] {
            let change = changed
                .changes
                .iter()
                .find(|change| change.mode == letter)
                .unwrap();
            assert_eq!(change.action, ModeAction::Add);
            assert!(matches!(change.arg.clone(), Some(ModeArg::Nick(value)) if value == *nick));
        }
    }
    for peer in [&alice, &bob, &carol, &dave] {
        let state = peer.state();
        let members = state.channel(&channel).expect("channel is in state");
        assert_eq!(members.member(&bob_nick).unwrap().prefixes(), &['@'][..]);
        assert_eq!(members.member(&carol_nick).unwrap().prefixes(), &['+'][..]);
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
    assert_eq!(received.status.as_deref(), Some(['@'].as_slice()));
    assert_eq!(received.raw_text, "operators only");
    assert_eq!(received.msgid.as_ref(), sent.msgid.as_ref());
    assert_eq!(received.origin, Origin::Live);
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
    covers = ["base.privmsg.statusmsg-reply-ops"]
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
    assert_eq!(question.status.as_deref(), Some(['@'].as_slice()));
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
    assert_eq!(answer.status.as_deref(), Some(['@'].as_slice()));
    for peer in [&mut carol, &mut dave] {
        peer.expect_none(|event| message_text(event, "operators-only answer"))
            .await?;
    }
    Ok(())
}
