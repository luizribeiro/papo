use papo_conformance_macros::scenario;

use crate::{RegisteredClient, Scenario, ScenarioResult, required_environment};

/// registers with the requested nickname
#[scenario]
async fn registration(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let connection = context.client(&nick).await?;
    validate_registration(connection, &nick).await
}

/// registers over TLS with the configured test CA
#[scenario]
async fn tls(context: Scenario) -> ScenarioResult {
    let address = format!("ircs://{}", context.server.tls_addr());
    register_over(context, &address).await
}

/// registers over a WebSocket transport
#[scenario]
async fn websocket(context: Scenario) -> ScenarioResult {
    register_over_environment(context, "PAPO_TEST_WS").await
}

/// registers over a TLS WebSocket transport
#[scenario]
async fn websocket_tls(context: Scenario) -> ScenarioResult {
    register_over_environment(context, "PAPO_TEST_WSS").await
}

async fn register_over_environment(context: Scenario, variable: &'static str) -> ScenarioResult {
    let address = match required_environment(variable) {
        Ok(address) => address,
        Err(ignored) => return Ok(ignored),
    };
    register_over(context, &address).await
}

async fn register_over(context: Scenario, address: &str) -> ScenarioResult {
    let nick = context.unique();
    let connection = context.client_at(&nick, address).await?;
    validate_registration(connection, &nick).await
}

async fn validate_registration(connection: RegisteredClient, nick: &str) -> ScenarioResult {
    if connection.welcome.nick != nick {
        return Err("welcome nickname differs from the requested nickname".into());
    }
    connection.close().await
}
