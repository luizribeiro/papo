use papo::{ModeKind, Refusal, Source};
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::{assert_refusal, join_seen_by, message_text, wait_channel_mode};

enum Dialect {
    ListMode,
    TildeExtban,
    MuteExtban,
}

impl Dialect {
    fn mode(self, mask: &str) -> (char, String, String) {
        match self {
            Self::ListMode => ('q', mask.to_owned(), mask.to_owned()),
            Self::TildeExtban => ('b', format!("~q:{mask}"), format!("~quiet:{mask}")),
            Self::MuteExtban => ('b', format!("m:{mask}"), format!("m:{mask}")),
        }
    }
}

fn pick_dialect(alice: &papo_testing::Peer) -> Option<Dialect> {
    let state = alice.state();
    let support = state.isupport();
    let has_q_list = support
        .chanmodes()
        .is_some_and(|modes| modes.list.contains('q'));
    let q_is_prefix = support.prefix().iter().any(|prefix| prefix.mode == 'q');
    if has_q_list && !q_is_prefix {
        return Some(Dialect::ListMode);
    }
    let extban = support.extban()?;
    if extban.prefix == Some('~') && extban.types.contains('q') {
        return Some(Dialect::TildeExtban);
    }
    if extban.prefix.is_none() && extban.types.contains('m') {
        return Some(Dialect::MuteExtban);
    }
    None
}

/// mutes a member through the server's advertised quiet dialect
#[scenario(covers = ["base.isupport.extban-quiet"])]
async fn quiet_by_dialect(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    let Some(dialect) = pick_dialect(&alice) else {
        return s.skip("server advertises neither a q list mode nor a mute extban");
    };

    let mut bob = s.connect("bob").await?;
    let mut carol = s.connect("carol").await?;
    let bob_nick = bob.nick();
    let carol_nick = carol.nick();
    let mask = format!("{carol_nick}!*@*");
    let (letter, arg, echoed_arg) = dialect.mode(&mask);

    alice.join(&channel).await?;
    join_seen_by(&bob, &mut alice, &channel).await?;
    join_seen_by(&carol, &mut alice, &channel).await?;

    alice.mode(&channel).add_with(letter, arg.clone()).await?;
    let on_bob = wait_channel_mode(&mut bob, &channel, letter).await?;
    let on_carol = wait_channel_mode(&mut carol, &channel, letter).await?;
    for changed in [&on_bob, &on_carol] {
        assert_eq!(changed.by.nick(), Some(alice_nick.as_ref()));
        assert_eq!(changed.changes.len(), 1);
        let change = &changed.changes[0];
        assert_eq!(change.mode, letter);
        assert!(change.set);
        assert_eq!(change.kind, ModeKind::List);
        assert_eq!(change.arg.as_deref(), Some(echoed_arg.as_str()));
    }

    let blocked = "muted members cannot speak";
    let error = carol.privmsg(&channel, blocked).await.unwrap_err();
    assert_refusal(error, Refusal::CannotSendToChannel, "404");
    alice
        .expect_none(|event| message_text(event, blocked))
        .await?;
    bob.expect_none(|event| message_text(event, blocked))
        .await?;

    let unmuted = "unmuted members still speak";
    bob.privmsg(&channel, unmuted).await?;
    for peer in [&mut alice, &mut carol] {
        let message = peer.wait_for(|event| message_text(event, unmuted)).await?;
        assert_eq!(message.target(), channel);
        assert_eq!(
            message.sender.as_ref().and_then(Source::nick),
            Some(bob_nick.as_ref())
        );
        assert!(!message.is_private());
    }
    for peer in [&alice, &bob, &carol] {
        let state = peer.state();
        let member = state
            .channel(&channel)
            .unwrap()
            .member(&carol_nick)
            .unwrap();
        assert!(member.prefixes().is_empty());
    }

    alice.mode(&channel).remove_with(letter, arg).await?;
    let removed = wait_channel_mode(&mut bob, &channel, letter).await?;
    assert_eq!(removed.changes.len(), 1);
    let change = &removed.changes[0];
    assert_eq!(change.mode, letter);
    assert!(!change.set);
    assert_eq!(change.kind, ModeKind::List);

    let again = "unmuted again";
    carol.privmsg(&channel, again).await?;
    let message = bob.wait_for(|event| message_text(event, again)).await?;
    assert_eq!(
        message.sender.as_ref().and_then(Source::nick),
        Some(carol_nick.as_ref())
    );
    Ok(())
}
