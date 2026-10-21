use papo::isupport::{Advertised, Prefix};
use papo::{ModeAction, ModeArg, Nick, Rank, State};
use papo_conformance_macros::scenario;
use papo_test_server::DeploymentProfile;
use papo_testing::Peer;

use crate::Scenario;
use crate::scenarios::support::{channel_mode, join_seen_by, wait_channel_mode};

/// grants every status mode an operator may set and reports member prefixes in rank order
#[scenario(
    requires(caps = ["multi-prefix"]),
    covers = [
        "base.isupport.prefix",
        "base.isupport.prefix-rank-order",
        "ircv3.multi-prefix"
    ]
)]
async fn prefix_member_ranks(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let (alice_nick, bob_nick) = (alice.nick(), bob.nick());
    let channel = s.channel_name();
    alice.join(&channel).await?;
    join_seen_by(&bob, &mut alice, &channel).await?;

    // Parsed by hand from the raw token, independently of `Isupport::prefix`'s own parser.
    let expected = parse_prefix_token(alice.state().isupport());
    assert_eq!(alice.state().isupport().prefix(), expected);
    let allowed = allowed_prefixes(&expected);
    let allowed_symbols: Vec<char> = allowed.iter().map(|prefix| prefix.symbol).collect();
    let has_halfop = allowed.iter().any(|prefix| prefix.mode == 'h');
    let expected_symbols: &[char] = match DeploymentProfile::from_env()? {
        DeploymentProfile::ErgoDefault
        | DeploymentProfile::ErgoPersistent
        | DeploymentProfile::SojuErgo
        | DeploymentProfile::UnrealIrcd => &['@', '%', '+'],
        DeploymentProfile::InspIrcd | DeploymentProfile::Solanum => &['@', '+'],
    };
    assert_eq!(allowed_symbols, expected_symbols);
    assert_eq!(
        member_prefixes(&alice.state(), &channel, &alice_nick),
        ['@']
    );

    // Reverse rank order so the final state can't be mistaken for insertion order.
    for prefix in allowed.iter().rev() {
        grant_rank(&alice, &mut bob, &channel, &alice_nick, &bob_nick, *prefix).await?;
    }
    for peer in [&alice, &bob] {
        assert_eq!(
            member_prefixes(&peer.state(), &channel, &bob_nick),
            allowed_symbols
        );
    }

    assert_carol_view(&s, &channel, &alice_nick, &bob_nick).await?;
    assert_dave_view(&s, &channel, &bob_nick, &allowed_symbols, has_halfop).await?;

    bob.expect_none(|event| channel_mode(event, &channel))
        .await?;
    Ok(())
}

fn parse_prefix_token(support: &papo::Isupport) -> Vec<Prefix> {
    let Advertised::Value(raw) = support.get("PREFIX") else {
        panic!(
            "PREFIX must advertise a (modes)symbols value, got {:?}",
            support.get("PREFIX")
        );
    };
    let (modes, symbols) = raw
        .strip_prefix('(')
        .and_then(|rest| rest.split_once(')'))
        .unwrap_or_else(|| panic!("PREFIX value {raw:?} is not (modes)symbols"));
    assert_eq!(modes.chars().count(), symbols.chars().count());
    modes
        .chars()
        .zip(symbols.chars())
        .enumerate()
        .map(|(rank, (mode, symbol))| Prefix { mode, symbol, rank })
        .collect()
}

fn allowed_prefixes(expected: &[Prefix]) -> Vec<Prefix> {
    let op_rank = expected
        .iter()
        .find(|prefix| prefix.symbol == '@')
        .expect("the operator prefix @ must be advertised")
        .rank;
    expected
        .iter()
        .copied()
        .filter(|prefix| prefix.rank >= op_rank)
        .collect()
}

fn member_prefixes(state: &State, channel: &str, nick: &str) -> Vec<char> {
    state
        .channel(channel)
        .unwrap()
        .member(nick)
        .unwrap()
        .prefixes()
        .to_vec()
}

async fn grant_rank(
    alice: &Peer,
    bob: &mut Peer,
    channel: &str,
    alice_nick: &Nick,
    bob_nick: &Nick,
    prefix: Prefix,
) -> papo_testing::Result<()> {
    if prefix.mode == 'h' {
        alice.mode(channel).halfop(bob_nick).await?;
    } else {
        alice
            .mode(channel)
            .add_with(prefix.mode, bob_nick.to_string())
            .await?;
    }
    let changed = wait_channel_mode(bob, channel, prefix.mode).await?;
    assert_eq!(
        crate::source_nick(&changed.actor),
        Some(alice_nick.as_ref())
    );
    assert_eq!(changed.changes.len(), 1);
    let change = &changed.changes[0];
    assert_eq!(change.action, ModeAction::Add);
    assert_eq!(change.mode, prefix.mode);
    assert!(matches!(change.arg.clone(), Some(ModeArg::Nick(nick)) if nick == *bob_nick));
    Ok(())
}

async fn assert_carol_view(
    s: &Scenario,
    channel: &str,
    alice_nick: &Nick,
    bob_nick: &Nick,
) -> papo_testing::Result<()> {
    let carol = s
        .connect_with(
            s.configure("carol")?
                .without_caps([papo::cap::MULTI_PREFIX]),
        )
        .await?;
    assert!(!carol.state().caps().contains_key(&papo::cap::MULTI_PREFIX));
    carol.join(channel).await?;
    let carol_nick = carol.nick();
    assert_eq!(member_prefixes(&carol.state(), channel, bob_nick), ['@']);
    assert_eq!(member_prefixes(&carol.state(), channel, alice_nick), ['@']);
    assert!(member_prefixes(&carol.state(), channel, &carol_nick).is_empty());
    Ok(())
}

async fn assert_dave_view(
    s: &Scenario,
    channel: &str,
    bob_nick: &Nick,
    allowed_symbols: &[char],
    has_halfop: bool,
) -> papo_testing::Result<()> {
    let dave = s.connect("dave").await?;
    assert!(dave.state().caps().contains_key(&papo::cap::MULTI_PREFIX));
    dave.join(channel).await?;
    let state = dave.state();
    let member = state.channel(channel).unwrap().member(bob_nick).unwrap();
    assert_eq!(member.prefixes(), allowed_symbols);
    assert!(member.rank() >= Some(Rank::Op));
    assert!(member.has(Rank::Op));
    assert!(member.has(Rank::Voice));
    assert_eq!(member.has(Rank::Halfop), has_halfop);
    assert_eq!(member.prefixes().contains(&'%'), has_halfop);
    Ok(())
}
