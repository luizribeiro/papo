use std::collections::HashSet;

use papo::{Error, InvalidInput, Isupport, ModeAction, isupport::Advertised};
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::{channel_mode, join_seen_by, wait_channel_mode};

/// rejects unadvertised mode letters and class-mismatched arguments before sending
#[scenario(covers = ["base.isupport.chanmodes-local-validation"])]
async fn unadvertised_mode_rejected(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    join_seen_by(&bob, &mut alice, &channel).await?;

    let letter = unadvertised_mode_letter(alice.state().isupport())
        .expect("CHANMODES and PREFIX leave at least one Latin letter unadvertised");

    assert_invalid_message(&alice.mode(&channel).add(letter).await);
    assert_invalid_message(&alice.mode(&channel).add_with(letter, "review").await);
    assert_invalid_message(&alice.mode(&channel).add('k').await);
    assert_invalid_message(&alice.mode(&channel).remove_with('l', "10").await);
    assert_invalid_message(&alice.mode(&channel).add('b').await);
    assert_invalid_message(&alice.mode(&channel).add_with('n', "x").await);
    assert_invalid_message(&alice.mode(&channel).await);
    assert_invalid_message(&alice.mode(&channel).add('m').add(letter).await);

    bob.expect_none(|event| channel_mode(event, &channel))
        .await?;
    alice
        .expect_none(|event| channel_mode(event, &channel))
        .await?;
    assert_eq!(alice.state().channel(&channel).unwrap().mode('m'), None);
    assert_eq!(alice.state().channel(&channel).unwrap().mode(letter), None);

    let alice_nick = alice.nick();
    alice.mode(&channel).add('m').await?;
    for peer in [&mut alice, &mut bob] {
        let changed = wait_channel_mode(peer, &channel, 'm').await?;
        assert_eq!(
            crate::source_nick(&changed.actor),
            Some(alice_nick.as_ref())
        );
        assert_eq!(changed.changes.len(), 1);
        let applied = &changed.changes[0];
        assert_eq!(applied.action, ModeAction::Add);
        assert_eq!(applied.arg, None);
    }
    for peer in [&alice, &bob] {
        let state = peer.state();
        let channel_state = state.channel(&channel).unwrap();
        assert_eq!(channel_state.mode('m'), Some(None));
        assert_eq!(channel_state.mode(letter), None);
    }
    Ok(())
}

fn assert_invalid_message<T: std::fmt::Debug>(result: &Result<T, Error>) {
    let Err(Error::InvalidInput(InvalidInput::InvalidMessage(_))) = result else {
        panic!("expected Error::InvalidInput(InvalidInput::InvalidMessage(_)), got {result:?}");
    };
}

fn unadvertised_mode_letter(support: &Isupport) -> Option<char> {
    let Advertised::Value(chanmodes) = support.get("CHANMODES") else {
        return None;
    };
    let Advertised::Value(prefix) = support.get("PREFIX") else {
        return None;
    };
    let prefix_modes = prefix
        .strip_prefix('(')
        .and_then(|rest| rest.split(')').next())
        .unwrap_or("");
    let advertised: HashSet<char> = chanmodes
        .chars()
        .chain(prefix_modes.chars())
        .filter(|&letter| letter != ',')
        .collect();
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
        .chars()
        .find(|letter| !advertised.contains(letter))
}
