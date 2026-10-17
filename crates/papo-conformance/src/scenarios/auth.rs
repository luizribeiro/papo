use libtest_mimic::Failed;
use papo::{AccountRegistration, Event};
use papo_conformance_macros::scenario;
use papo_core::Sasl;
use papo_test_server::register_account;

use super::support::{EXPECT_TIMEOUT, PASSWORD};
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
    let mut connection = context.client_with(&nick, Some(sasl)).await?;
    let registered_nick = connection.welcome.nick.clone();
    connection
        .client
        .send_raw(&format!("WHOIS {registered_nick}"))
        .await
        .map_err(Failed::from)?;
    let event = context
        .expect(
            &mut connection.events,
            |event| match event {
                Event::Raw(message) => {
                    message.command().eq_ignore_ascii_case("900")
                        || message.command().eq_ignore_ascii_case("330")
                }
                _ => false,
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    let Event::Raw(message) = event else {
        return Err("account matcher returned a non-message event".into());
    };
    validate_account_reply(message.command(), message.params(), &account)?;
    connection.close().await
}

fn validate_account_reply(command: &str, params: &[String], account: &str) -> Result<(), Failed> {
    if (command.eq_ignore_ascii_case("900") || command.eq_ignore_ascii_case("330"))
        && params.get(2).is_some_and(|actual| actual == account)
    {
        Ok(())
    } else {
        Err("server account reply did not contain the authenticated account".into())
    }
}

/// registers and authenticates a new account
#[scenario(requires(caps = ["draft/account-registration"]))]
async fn draft_account_registration(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let account = nick.clone();
    let mut connection = context
        .client_caps(&nick, &["draft/account-registration"])
        .await?;
    connection
        .client
        .send_raw(&format!("REGISTER {account} * {PASSWORD}"))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| match event {
                Event::AccountRegistration(AccountRegistration::Registered {
                    account: registered,
                }) => registered == &account,
                Event::Raw(message) => {
                    message.command().eq_ignore_ascii_case("REGISTER")
                        && message
                            .params()
                            .first()
                            .is_some_and(|value| value == "SUCCESS")
                        && message
                            .params()
                            .get(1)
                            .is_some_and(|value| value == &account)
                }
                _ => false,
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    connection.close().await
}

#[cfg(test)]
mod tests {
    use super::validate_account_reply;

    #[test]
    fn wrong_account_in_logged_in_reply_fails() {
        let params = [
            "papo".to_owned(),
            "papo!~u@host".to_owned(),
            "mallory".to_owned(),
            "You are now logged in as mallory".to_owned(),
        ];
        let error = validate_account_reply("900", &params, "alice").unwrap_err();
        assert_eq!(
            error.message(),
            Some("server account reply did not contain the authenticated account")
        );
    }
}
