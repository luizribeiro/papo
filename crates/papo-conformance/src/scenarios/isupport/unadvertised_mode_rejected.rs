use std::collections::HashSet;

use papo::{Error, Event, InvalidInput, Isupport, ModeKind, isupport::Advertised};
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::{channel_mode, join_seen_by};

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

    alice.mode(&channel).add('m').await?;
    let change_event = bob.wait_for(Event::as_mode_changed).await?;
    assert_eq!(change_event.target.as_ref(), channel);
    assert_eq!(change_event.changes.len(), 1);
    let applied = &change_event.changes[0];
    assert_eq!(applied.mode, 'm');
    assert!(applied.set);
    assert_eq!(applied.kind, ModeKind::NoArg);
    assert_eq!(applied.arg, None);
    Ok(())
}

fn assert_invalid_message(result: &Result<(), Error>) {
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
        .filter(|&letter| letter != ':')
        .collect();
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
        .chars()
        .find(|letter| !advertised.contains(letter))
}
