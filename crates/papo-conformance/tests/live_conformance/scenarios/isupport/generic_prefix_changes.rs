use papo::{ModeAction, ModeArg};
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::wait_channel_mode;

/// round-trips membership modes through their advertised letters
#[scenario(covers = ["papo.moderation.generic-prefix"])]
async fn generic_prefix_changes(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let bob_nick = bob.snapshot().nick().clone();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let joined = alice
        .wait_for(|event| {
            event
                .as_joined()
                .filter(|join| join.actor.nick() == bob_nick.as_str())
        })
        .await?;

    let prefixes = alice.snapshot().isupport().prefix();
    let operator = prefixes
        .iter()
        .position(|prefix| prefix.symbol() == '@')
        .expect("server advertises the channel operator prefix");
    for prefix in &prefixes[operator..] {
        let letter = prefix.mode();
        let grant = alice
            .set_modes(&channel)
            .add_with(letter, &joined.actor)
            .await?;
        let granted = wait_channel_mode(&mut bob, &channel, letter).await?;
        let revoke = alice
            .set_modes(&channel)
            .remove_with(letter, bob_nick.as_str())
            .await?;
        let revoked = wait_channel_mode(&mut bob, &channel, letter).await?;
        for (outcome, event, action) in [
            (grant, granted, ModeAction::Add),
            (revoke, revoked, ModeAction::Remove),
        ] {
            assert_eq!(outcome.applied.len(), 1);
            assert!(outcome.refused.is_empty());
            assert!(outcome.failed.is_empty());
            assert!(matches!(
                event.changes.as_slice(),
                [change] if change.action == action && change.mode == letter
                    && matches!(&change.arg, Some(ModeArg::Member { user, prefix: actual, .. })
                        if user.nick() == bob_nick.as_str() && actual == prefix)
            ));
        }
    }
    Ok(())
}
