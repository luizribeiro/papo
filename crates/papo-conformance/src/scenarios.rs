use std::time::Duration;

use libtest_mimic::Failed;
use papo::Event;
use papo_core::Sasl;
use papo_proto::Message;
use papo_test_server::register_account;

use super::{CapRequirement, Requirements, Scenario, ScenarioFuture, ScenarioResult, ScenarioSpec};

const EXPECT_TIMEOUT: Duration = Duration::from_secs(10);
const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

pub(super) const fn all() -> &'static [ScenarioSpec] {
    &[
        ScenarioSpec {
            name: "registration",
            doc: "registers with the requested nickname",
            requires: Requirements {
                caps: &[],
                isupport: &[],
            },
            run: registration,
        },
        ScenarioSpec {
            name: "sasl_plain",
            doc: "authenticates an account with SASL PLAIN",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "sasl",
                    value: None,
                }],
                isupport: &[],
            },
            run: sasl_plain,
        },
        ScenarioSpec {
            name: "sasl_scram_sha_256",
            doc: "authenticates an account with SCRAM-SHA-256",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "sasl",
                    value: Some("SCRAM-SHA-256"),
                }],
                isupport: &[],
            },
            run: sasl_scram_sha_256,
        },
    ]
}

fn registration(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let nick = context.unique();
        let connection = context.client(&nick).await?;
        if connection.welcome.nick != nick {
            return Err("welcome nickname differs from the requested nickname".into());
        }
        connection.close().await
    })
}

fn sasl_plain(context: Scenario) -> ScenarioFuture {
    Box::pin(authenticate(context, false))
}

fn sasl_scram_sha_256(context: Scenario) -> ScenarioFuture {
    Box::pin(authenticate(context, true))
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
    context
        .expect(
            &mut connection.events,
            |event| match event {
                Event::Message(message) => {
                    let message = message.as_ref();
                    message.command_is("330") && message.param(2) == Some(account.as_str())
                }
                _ => false,
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    connection.close().await
}
