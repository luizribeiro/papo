use libtest_mimic::Failed;
use papo::{AccountRegistration, NewAccount};
use papo_conformance_macros::scenario;
use papo_core::Sasl;
use papo_test_server::register_account;

use super::support::PASSWORD;
use crate::{Scenario, ScenarioResult};

/// authenticates an account with SASL PLAIN
#[scenario(requires(caps = ["sasl"]))]
async fn sasl_plain(context: Scenario) -> ScenarioResult {
    authenticate(context, false).await
}

/// authenticates an account with SCRAM-SHA-256
#[scenario(requires(caps = ["sasl=SCRAM-SHA-256"]))]
async fn sasl_scram_sha_256(context: Scenario) -> ScenarioResult {
    authenticate(context, true).await
}

async fn authenticate(context: Scenario, scram: bool) -> ScenarioResult {
    let account = context.unique();
    register_account(&context.server, &account, PASSWORD).map_err(Failed::from)?;
    let sasl = if scram {
        Sasl::scram_sha256(&account, PASSWORD)
    } else {
        Sasl::plain(&account, PASSWORD)
    };
    let nick = context.unique();
    let connection = context.client_with(&nick, Some(sasl)).await?;
    let registered_nick = connection.welcome.nick.clone();
    let whois = connection
        .client
        .whois(&registered_nick)
        .await
        .map_err(Failed::from)?;
    if whois.account.as_deref() != Some(account.as_str()) {
        return Err("WHOIS did not contain the authenticated account".into());
    }
    connection.close().await
}

/// registers and authenticates a new account
#[scenario(requires(caps = ["draft/account-registration"]))]
async fn draft_account_registration(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let account = nick.clone();
    let connection = context
        .client_caps(&nick, &["draft/account-registration"])
        .await?;
    let registered = connection
        .client
        .register_account(NewAccount::new(PASSWORD).account(&account))
        .await
        .map_err(Failed::from)?;
    if !matches!(registered, AccountRegistration::Registered { account: registered } if registered.as_ref() == account)
    {
        return Err("REGISTER did not return the new account".into());
    }
    connection.close().await
}
