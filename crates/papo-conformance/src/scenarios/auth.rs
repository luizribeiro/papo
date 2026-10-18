use papo::{AccountName, AccountRegistration, Event, NewAccount};
use papo_conformance_macros::scenario;
use papo_core::Sasl;
use papo_test_server::{register_account, unique};

use super::support::PASSWORD;
use crate::Scenario;

/// authenticates an account with SASL PLAIN
#[scenario(
    requires(caps = ["sasl"]),
    covers = [
        "base.whois.user",
        "base.whois.account",
        "ircv3.sasl.plain",
        "ircv3.sasl.logged-in-900"
    ]
)]
async fn sasl_plain(s: Scenario) -> papo_testing::Result<()> {
    authenticate(s, false).await
}

/// authenticates an account with SCRAM-SHA-256
#[scenario(
    requires(caps = ["sasl=SCRAM-SHA-256"]),
    covers = [
        "base.whois.user",
        "base.whois.account",
        "ircv3.sasl.scram-sha-256",
        "ircv3.sasl.logged-in-900"
    ]
)]
async fn sasl_scram_sha_256(s: Scenario) -> papo_testing::Result<()> {
    authenticate(s, true).await
}

async fn authenticate(s: Scenario, scram: bool) -> papo_testing::Result<()> {
    let account = unique("alice-account");
    register_account(&s.server, &account, PASSWORD)?;
    let sasl = if scram {
        Sasl::scram_sha256(&account, PASSWORD)
    } else {
        Sasl::plain(&account, PASSWORD)
    };
    let mut alice = s.connect_with(s.configure("alice").sasl(sasl)).await?;
    let registered = alice.wait_for(Event::as_registered).await?;
    assert_eq!(&registered.nick, alice.state().nick());
    assert!(registered.caps.contains_key("sasl"));
    assert_eq!(
        alice.state().account().map(AsRef::as_ref),
        Some(account.as_str())
    );

    let whois = alice.whois(alice.state().nick().clone()).await?;
    assert_eq!(whois.nick, *alice.state().nick());
    assert_eq!(whois.account.as_deref(), Some(account.as_str()));
    alice.expect_none(Event::as_account_changed).await?;
    Ok(())
}

/// registers and authenticates a new account
#[scenario(
    requires(caps = ["draft/account-registration"]),
    covers = ["draft.account-registration.register"]
)]
async fn draft_account_registration(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s
        .connect_with(
            s.configure("alice")
                .request_caps(["draft/account-registration"]),
        )
        .await?;
    let nick = alice.state().nick().clone();
    let account = AccountName::from(nick.as_ref());
    let outcome = alice
        .register_account(NewAccount::new(PASSWORD).account(account.clone()))
        .await?;
    assert!(matches!(
        &outcome,
        AccountRegistration::Registered { account: registered } if registered == &account
    ));
    assert_eq!(
        alice.wait_for(Event::as_account_registration).await?,
        outcome
    );
    assert_eq!(alice.state().account(), Some(&account));

    let whois = alice.whois(nick.clone()).await?;
    assert_eq!(whois.nick, nick);
    assert_eq!(whois.account, Some(account));
    alice.expect_none(Event::as_account_registration).await?;
    Ok(())
}
