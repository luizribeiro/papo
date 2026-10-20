use papo::{Error, LegacyModeAction as ModeAction, LegacyModeArg as ModeArg, Refusal};
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::{channel_mode, join_seen_by, wait_channel_mode};

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
    join_seen_by(&bob, &mut alice, &channel).await?;
    join_seen_by(&carol, &mut alice, &channel).await?;

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
    bob.expect_none(|event| channel_mode(event, &channel))
        .await?;
    carol
        .expect_none(|event| channel_mode(event, &channel))
        .await?;

    alice.mode(&channel).op(bob_nick.clone()).await?;
    for peer in [&mut alice, &mut bob, &mut carol] {
        let promoted = wait_channel_mode(peer, &channel, 'o').await?;
        assert_eq!(
            crate::source_nick(&promoted.actor),
            Some(alice_nick.as_ref())
        );
        assert_eq!(promoted.changes.len(), 1);
        let change = &promoted.changes[0];
        assert_eq!(change.action, ModeAction::Add);
        assert!(matches!(&change.typed_arg, Some(ModeArg::Nick(nick)) if nick == &bob_nick));
    }
    for peer in [&alice, &bob, &carol] {
        let state = peer.state();
        let member = state.channel(&channel).unwrap().member(&bob_nick).unwrap();
        assert_eq!(member.prefixes(), &['@'][..]);
    }
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
