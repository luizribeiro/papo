use libtest_mimic::Failed;
use papo::Event;
use papo_conformance_macros::scenario;
use papo_core::Sasl;
use papo_proto::Message;
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
        .send(Message::new("WHOIS", [registered_nick]))
        .await
        .map_err(Failed::from)?;
    let event = context
        .expect(
            &mut connection.events,
            |event| match event {
                Event::Raw(message) => {
                    let message = message.as_ref();
                    message.command_is("900") || message.command_is("330")
                }
                _ => false,
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    let Event::Raw(message) = event else {
        return Err("account matcher returned a non-message event".into());
    };
    super::validate_account_reply(&message.as_ref(), &account)?;
    connection.close().await
}
