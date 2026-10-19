use papo::{Error, Event, ModeKind, Refusal};
use papo_conformance_macros::scenario;

use crate::Scenario;

/// refuses a channel operator granting a status mode ranked above its own
#[scenario(covers = [
    "base.isupport.prefix",
    "base.mode.channel.err-prefix-above-rank"
])]
async fn prefix_above_rank_refused(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let mut carol = s.connect("carol").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    alice.wait_for(Event::as_joined).await?;
    carol.join(&channel).await?;
    alice.wait_for(Event::as_joined).await?;

    let prefixes = alice.state().isupport().prefix();
    let operator_rank = prefixes
        .iter()
        .find(|prefix| prefix.symbol == '@')
        .expect("PREFIX must advertise the channel-operator symbol")
        .rank;
    let above: Vec<_> = prefixes
        .iter()
        .copied()
        .filter(|prefix| prefix.rank < operator_rank)
        .collect();
    if above.is_empty() {
        return s.skip("PREFIX advertises no status mode above channel operator");
    }

    for prefix in &above {
        let error = alice
            .mode(&channel)
            .add_with(prefix.mode, bob_nick.to_string())
            .await
            .unwrap_err();
        assert_above_rank_refusal(error);
    }

    for peer in [&alice, &bob, &carol] {
        let state = peer.state();
        let channel_state = state.channel(&channel).unwrap();
        assert!(
            channel_state
                .member(&bob_nick)
                .unwrap()
                .prefixes()
                .is_empty()
        );
        assert_eq!(
            channel_state.member(&alice_nick).unwrap().prefixes(),
            &['@'][..]
        );
    }
    bob.expect_none(|event| {
        event
            .as_mode_changed()
            .filter(|changed| changed.target.as_ref() == channel)
    })
    .await?;
    carol
        .expect_none(|event| {
            event
                .as_mode_changed()
                .filter(|changed| changed.target.as_ref() == channel)
        })
        .await?;

    alice.mode(&channel).op(bob_nick.clone()).await?;
    let promoted = carol
        .wait_for(|event| {
            event
                .as_mode_changed()
                .filter(|changed| changed.target.as_ref() == channel)
        })
        .await?;
    let change = promoted
        .changes
        .iter()
        .find(|change| change.mode == 'o')
        .unwrap();
    assert!(change.set);
    assert_eq!(change.kind, ModeKind::Prefix(operator_rank));
    assert_eq!(change.arg.as_deref(), Some(bob_nick.as_ref()));
    Ok(())
}

fn assert_above_rank_refusal(error: Error) {
    let Error::Server(refusal) = error else {
        panic!("granting an above-rank status mode did not return a server refusal: {error:?}");
    };
    let expected = match refusal.code() {
        "482" => Refusal::ChannelOperatorPrivilegesNeeded,
        "499" => Refusal::Other,
        other => {
            panic!("unexpected refusal code {other:?} for an above-rank status mode: {refusal:?}")
        }
    };
    assert_eq!(refusal.kind(), expected);
}
