use papo::ModeKind;
use papo::event::ModeChanged;
use papo::isupport::Advertised;
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::{channel_mode, join_seen_by};

/// splits a mode request into lines of at most MODES changes
#[scenario(covers = ["base.isupport.modes"])]
async fn modes_limit(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    join_seen_by(&bob, &mut alice, &channel).await?;

    let support = alice.state().isupport().clone();
    let limit = match support.get("MODES") {
        Advertised::Value(value) => Some(value.parse::<usize>().expect("MODES value is numeric")),
        Advertised::Bare => None,
        Advertised::Absent => return s.skip("server does not advertise MODES"),
    };
    assert_eq!(support.modes(), limit);

    let count = limit.map_or(5, |n| n + 1);
    // Short enough that a chunk at InspIRCd's MODES=20, the largest among the
    // tested servers, still fits a 512-byte line alongside a full-length
    // generated channel name.
    let masks = (1..=count)
        .map(|i| format!("flood{i}!*@{i}"))
        .collect::<Vec<_>>();
    let request = masks
        .iter()
        .cloned()
        .fold(alice.mode(&channel), papo::SetModes::ban);
    request.await?;

    let assert_ban_changes = |changed: &ModeChanged, expected: &[String]| {
        assert_eq!(changed.target.as_ref(), channel);
        assert_eq!(changed.changes.len(), expected.len());
        for (change, mask) in changed.changes.iter().zip(expected) {
            assert_eq!(change.mode, 'b');
            assert!(change.set);
            assert_eq!(change.kind, ModeKind::List);
            assert_eq!(change.arg.as_deref(), Some(mask.as_str()));
        }
    };

    if let Some(max) = limit {
        let first = bob.wait_for(|event| channel_mode(event, &channel)).await?;
        assert_ban_changes(&first, &masks[..max]);

        let second = bob.wait_for(|event| channel_mode(event, &channel)).await?;
        assert_ban_changes(&second, &masks[max..]);
    } else {
        let changed = bob.wait_for(|event| channel_mode(event, &channel)).await?;
        assert_ban_changes(&changed, &masks);
    }

    bob.expect_none(|event| channel_mode(event, &channel))
        .await?;
    Ok(())
}
