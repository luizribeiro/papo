use papo::{Error, Feature, Refusal};
use papo_conformance_macros::scenario;

use crate::Scenario;

/// refuses another list entry at the advertised MAXLIST boundary
#[scenario(covers = ["base.mode.channel.ban"])]
async fn maxlist_limit(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;

    let state = alice.state();
    let maximum = state
        .isupport()
        .maxlist()
        .into_iter()
        .find(|limit| limit.name.contains('b'))
        .and_then(|limit| limit.limit);
    let Some(maximum) = maximum else {
        return s.skip("server does not advertise a finite MAXLIST for bans");
    };
    let shared_quiet_list = state.supports(Feature::Quiet)
        && state
            .isupport()
            .maxlist()
            .iter()
            .any(|limit| limit.name.contains('b') && limit.name.contains('q'));
    let chunk_size = state.isupport().modes().unwrap_or(8).clamp(1, 8);
    let mapping = state.casemapping();
    drop(state);

    let masks = (0..maximum)
        .map(|index| mapping.mask(format!("flood-{index}!*@gateway-{index}.example")))
        .collect::<Vec<_>>();
    for (chunk_index, chunk) in masks.chunks(chunk_size).enumerate() {
        let mut request = alice.mode(&channel);
        for (offset, mask) in chunk.iter().enumerate() {
            request = if shared_quiet_list && chunk_index == 0 && offset == 0 {
                request.quiet(mask)
            } else {
                request.ban(mask)
            };
        }
        let outcome = request.await?;
        assert_eq!(outcome.applied.len(), chunk.len());
        assert!(outcome.refused.is_empty());
    }

    let error = alice
        .mode(&channel)
        .ban("overflow!*@gateway.example")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Server(error)
            if error.numeric() == Some(478) && error.refusal() == Refusal::BanListFull
    ));
    Ok(())
}
