use papo::ModeKind;
use papo::event::{ModeChange, ModeChanged};
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::{channel_mode, join_seen_by, wait_channel_mode};

const MASK: &str = "flooder!*@spam.example";
const KEY: &str = "review-key-2026";
const LIMIT: &str = "10";

/// pairs arguments with one mode from each CHANMODES class in a mixed mode change
#[scenario(covers = [
    "base.isupport.chanmodes-groups",
    "base.mode.channel.mixed-classes"
])]
async fn chanmode_classes(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    alice.join(&channel).await?;
    join_seen_by(&bob, &mut alice, &channel).await?;

    let support = alice.state().isupport().clone();
    let groups = support.chanmodes().expect("CHANMODES has four groups");
    assert!(groups.list.contains('b'));
    assert!(groups.always.contains('k'));
    assert!(groups.when_set.contains('l'));
    assert!(groups.never.contains('n'));

    alice
        .mode(&channel)
        .ban(alice.state().casemapping().mask(MASK))
        .add_with('k', KEY)
        .add_with('l', LIMIT)
        .remove('n')
        .await?;
    let changed = wait_channel_mode(&mut bob, &channel, 'k').await?;
    assert_eq!(changed.target.as_str(), channel);
    assert_eq!(changed.by.nick(), Some(alice_nick.as_ref()));
    assert_eq!(changed.changes.len(), 4);

    // Servers reorder the applied mode string on the wire (UnrealIRCd turns an
    // input of +bklm-n into +m-n+bkl), so changes are matched by letter, never index.
    let ban = find_change(&changed, 'b');
    assert!(ban.set);
    assert_eq!(ban.kind, ModeKind::List);
    assert_eq!(ban.arg.as_deref(), Some(MASK));
    let key = find_change(&changed, 'k');
    assert!(key.set);
    assert_eq!(key.kind, ModeKind::AlwaysArg);
    assert_eq!(key.arg.as_deref(), Some(KEY));
    let limit = find_change(&changed, 'l');
    assert!(limit.set);
    assert_eq!(limit.kind, ModeKind::ArgWhenSet);
    assert_eq!(limit.arg.as_deref(), Some(LIMIT));
    let no_external = find_change(&changed, 'n');
    assert!(!no_external.set);
    assert_eq!(no_external.kind, ModeKind::NoArg);
    assert_eq!(no_external.arg, None);

    for peer in [&alice, &bob] {
        let state = peer.state();
        let channel_state = state.channel(&channel).unwrap();
        assert_eq!(channel_state.mode('k'), Some(Some(KEY)));
        assert_eq!(channel_state.mode('l'), Some(Some(LIMIT)));
        assert_eq!(channel_state.mode('n'), None);
        assert_eq!(channel_state.mode('b'), None);
    }

    alice
        .mode(&channel)
        .unban(alice.state().casemapping().mask(MASK))
        .remove_with('k', KEY)
        .remove('l')
        .await?;
    let removed = wait_channel_mode(&mut bob, &channel, 'l').await?;
    assert_eq!(removed.changes.len(), 3);

    let ban = find_change(&removed, 'b');
    assert!(!ban.set);
    assert_eq!(ban.kind, ModeKind::List);
    assert_eq!(ban.arg.as_deref(), Some(MASK));
    let key = find_change(&removed, 'k');
    assert!(!key.set);
    assert_eq!(key.kind, ModeKind::AlwaysArg);
    // The echoed key varies by server: Ergo, soju, and Solanum send "*" rather than
    // the real key, so only its presence is checked here, never its value.
    assert!(key.arg.as_deref().is_some_and(|value| !value.is_empty()));
    let limit = find_change(&removed, 'l');
    assert!(!limit.set);
    assert_eq!(limit.kind, ModeKind::ArgWhenSet);
    assert_eq!(limit.arg, None);

    for peer in [&alice, &bob] {
        let state = peer.state();
        let channel_state = state.channel(&channel).unwrap();
        assert_eq!(channel_state.mode('k'), None);
        assert_eq!(channel_state.mode('l'), None);
    }

    bob.expect_none(|event| channel_mode(event, &channel))
        .await?;
    Ok(())
}

fn find_change(changed: &ModeChanged, mode: char) -> &ModeChange {
    changed
        .changes
        .iter()
        .find(|change| change.mode == mode)
        .unwrap_or_else(|| panic!("no change for mode {mode:?} in {changed:?}"))
}
