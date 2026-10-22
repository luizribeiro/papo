use papo::{
    Nick,
    isupport::{Advertised, LimitValue},
};
use papo_conformance_macros::scenario;

use crate::scenarios::support::message_text;
use crate::{Peer, Scenario};

const RECIPIENT_NAMES: [&str; 5] = ["bob", "carol", "dave", "erin", "frank"];

/// delivers a PRIVMSG to exactly its TARGMAX targets and refuses the target beyond it
#[scenario(
    requires(isupport = ["TARGMAX"]),
    covers = ["base.isupport.targmax", "base.privmsg.err-too-many-targets"]
)]
async fn targmax_privmsg(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let alice_nick = alice.nick();
    let state = alice.state();
    let support = state.isupport();
    let Some(LimitValue::Limited(limit)) = support
        .targmax()
        .into_iter()
        .find(|entry| entry.name.eq_ignore_ascii_case("PRIVMSG"))
        .and_then(|entry| entry.limit)
    else {
        return s.skip("TARGMAX advertises no finite PRIVMSG limit");
    };

    // papo's parsed limit is cross-checked against a hand split of the raw wire value.
    let Advertised::Value(raw) = support.get("TARGMAX") else {
        return s.skip("TARGMAX has no raw value to cross-check");
    };
    let hand_limit = raw
        .split(',')
        .filter_map(|pair| pair.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("PRIVMSG"))
        .map(|(_, value)| {
            if value.is_empty() {
                None
            } else {
                value.parse::<usize>().ok()
            }
        });
    assert_eq!(hand_limit, Some(Some(limit)));

    let Some(names) = RECIPIENT_NAMES.get(..=limit) else {
        return s.skip(format!(
            "TARGMAX PRIVMSG limit {limit} exceeds the fixture's five recipient names"
        ));
    };
    let mut recipients = Vec::with_capacity(names.len());
    for &name in names {
        recipients.push(s.connect(name).await?);
    }

    let nicks = recipients
        .iter()
        .map(|peer| peer.nick())
        .collect::<Vec<_>>();
    let (within_nicks, over_nicks) = nicks.split_at(limit);
    let frank_nick = over_nicks[0].clone();
    let mapping = alice.state().casemapping();
    let targets = within_nicks
        .iter()
        .map(AsRef::as_ref)
        .collect::<Vec<_>>()
        .join(",");
    let (within_limit, over_limit) = recipients.split_at_mut(limit);
    let frank = &mut over_limit[0];

    // papo's public API has no multi-target PRIVMSG; the comma-joined wire form is the
    // behavior under test, so the raw command is sent directly.
    let at_limit_text = "at the target limit";
    alice
        .send_raw(papo::Line::new(
            "PRIVMSG",
            [targets.as_str(), at_limit_text],
        ))
        .await?;
    for recipient in within_limit.iter_mut() {
        expect_delivery(recipient, &alice_nick, at_limit_text).await?;
    }
    frank
        .expect_none(|event| message_text(event, at_limit_text))
        .await?;
    alice
        .expect_none(|event| event.as_other().filter(|line| line.numeric() == Some(407)))
        .await?;

    let over_limit_text = "over the target limit";
    let all_targets = format!("{targets},{frank_nick}");
    alice
        .send_raw(papo::Line::new(
            "PRIVMSG",
            [all_targets.as_str(), over_limit_text],
        ))
        .await?;
    for recipient in within_limit.iter_mut() {
        expect_delivery(recipient, &alice_nick, over_limit_text).await?;
    }
    let refusal = alice
        .wait_for(|event| {
            event.as_other().filter(|line| {
                line.numeric() == Some(407)
                    && line
                        .param(1)
                        .is_some_and(|nick| mapping.nick(nick) == frank_nick)
            })
        })
        .await?;
    assert_eq!(refusal.numeric(), Some(407));
    assert_eq!(
        mapping.nick(refusal.param(1).expect("407 includes the refused nickname")),
        frank_nick
    );
    frank
        .expect_none(|event| message_text(event, over_limit_text))
        .await?;
    Ok(())
}

async fn expect_delivery(
    recipient: &mut Peer,
    sender_nick: &Nick,
    text: &str,
) -> papo_testing::Result<()> {
    let message = recipient
        .wait_for(|event| message_text(event, text))
        .await?;
    assert_eq!(message.text, text);
    assert_eq!(
        crate::scenarios::support::conversation_name(&message.conversation),
        Some(sender_nick.as_ref())
    );
    assert!(matches!(
        message.conversation,
        papo::Conversation::Private(_)
    ));
    assert_eq!(
        crate::source_nick(&message.actor),
        Some(sender_nick.as_ref())
    );
    Ok(())
}
