use papo::{Origin, Source};
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::{channel_mode_letter, message_text};

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
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let mut carol = s.connect("carol").await?;
    let mut dave = s.connect("dave").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    let carol_nick = carol.nick();
    let dave_nick = dave.nick();

    alice.join(&channel).await?;
    for (peer, nick) in [
        (&mut bob, bob_nick.clone()),
        (&mut carol, carol_nick.clone()),
        (&mut dave, dave_nick),
    ] {
        peer.join(&channel).await?;
        let joined = alice
            .wait_for(|event| event.as_joined().filter(|joined| joined.nick == nick))
            .await?;
        assert_eq!(joined.channel.as_ref(), channel);
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
    bob.wait_for(|event| channel_mode_letter(event, &channel, 'v'))
        .await?;
    carol
        .wait_for(|event| channel_mode_letter(event, &channel, 'v'))
        .await?;
    let changed = dave
        .wait_for(|event| channel_mode_letter(event, &channel, 'v'))
        .await?;
    assert_eq!(changed.changes.len(), 2);
    for peer in [&alice, &bob, &carol, &dave] {
        let state = peer.state();
        let members = state.channel(&channel).expect("channel is in state");
        assert_eq!(members.member(&bob_nick).unwrap().prefixes(), &['@'][..]);
        assert_eq!(members.member(&carol_nick).unwrap().prefixes(), &['+'][..]);
    }

    let sent = alice
        .privmsg(format!("@{channel}"), "operators only")
        .await?;
    let echo = alice
        .wait_for(|event| message_text(event, "operators only"))
        .await?;
    let received = bob
        .wait_for(|event| message_text(event, "operators only"))
        .await?;
    for message in [&echo, &received] {
        assert_eq!(message.target(), channel);
        assert_eq!(message.status.as_deref(), Some(['@'].as_slice()));
        assert_eq!(message.raw_text(), "operators only");
        assert_eq!(message.id(), sent.id.as_ref());
    }
    assert_eq!(echo.origin(), Origin::Echo);
    assert!(!echo.is_private());
    assert_eq!(received.origin(), Origin::Live);
    assert_eq!(
        received.sender.as_ref().and_then(Source::nick),
        Some(alice_nick.as_ref())
    );

    for peer in [&mut carol, &mut dave] {
        peer.expect_none(|event| message_text(event, "operators only"))
            .await?;
    }
    Ok(())
}
