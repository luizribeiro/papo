use papo::Error;
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::message_text;

/// rejects a comma-separated recipient list before any recipient receives the text
#[scenario(covers = ["security.target.no-comma-multitarget"])]
async fn comma_separated_targets_are_rejected(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let mut carol = s.connect("carol").await?;
    let target = format!("{},{}", bob.nick(), carol.nick());
    let text = "private release credentials";

    assert!(matches!(
        alice.privmsg(target, text).await,
        Err(Error::InvalidInput(_))
    ));
    bob.expect_none(|event| message_text(event, text)).await?;
    carol.expect_none(|event| message_text(event, text)).await?;
    Ok(())
}
