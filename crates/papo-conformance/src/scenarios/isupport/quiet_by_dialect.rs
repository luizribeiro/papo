use papo::event::ModeChanged;
use papo::{Error, Event, Message, ModeKind, Refusal};
use papo_conformance_macros::scenario;

use crate::Scenario;

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
    bob.join(&channel).await?;
    let joined = alice
        .wait_for(|event| event.as_joined().filter(|joined| !joined.is_self))
        .await?;
    assert_eq!(joined.nick, bob_nick);
    assert!(!joined.is_self);
    carol.join(&channel).await?;
    let joined = alice
        .wait_for(|event| event.as_joined().filter(|joined| !joined.is_self))
        .await?;
    assert_eq!(joined.nick, carol_nick);
    assert!(!joined.is_self);

    alice.mode(&channel).add_with(letter, arg).await?;
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
    Ok(())
}

async fn wait_channel_mode(
    peer: &mut papo_testing::Peer,
    channel: &str,
    mode_: char,
) -> papo_testing::Result<ModeChanged> {
    peer.wait_for(|event| {
        event.as_mode_changed().filter(|changed| {
            changed.target.as_ref() == channel
                && changed.changes.iter().any(|change| change.mode == mode_)
        })
    })
    .await
}

fn assert_refusal(error: Error, expected: Refusal, code: &str) {
    let refusal = match error {
        Error::Server(refusal) => refusal,
        other => panic!("operation did not return a server refusal: {other:?}"),
    };
    assert_eq!(refusal.kind(), expected);
    assert_eq!(refusal.code(), code);
}

fn message_text<'event>(event: &'event Event, text: &str) -> Option<&'event Message> {
    event.as_message().filter(|message| message.text() == text)
}
