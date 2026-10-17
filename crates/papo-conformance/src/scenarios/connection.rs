use libtest_mimic::Failed;
use papo::Endpoint;
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
    let endpoint = Endpoint::tls("localhost", context.server.tls_addr().port());
    register_over(context, endpoint).await
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
    let endpoint = Endpoint::websocket(address);
    register_over(context, endpoint).await
}

async fn register_over(context: Scenario, endpoint: Endpoint) -> ScenarioResult {
    let nick = context.unique();
    let connection = context.client_at(&nick, endpoint).await?;
    validate_registration(connection, &nick).await
}

async fn validate_registration(connection: RegisteredClient, nick: &str) -> ScenarioResult {
    if connection.welcome.nick != nick {
        return Err("welcome nickname differs from the requested nickname".into());
    }
    connection.close().await
}

/// answers a client PING with the matching PONG
#[scenario(requires(caps = ["labeled-response"]))]
async fn ping(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let connection = context.client(&nick).await?;
    let token = context.unique();
    let response = connection
        .client
        .request_raw(format!("PING {token}"))
        .await
        .map_err(Failed::from)?;
    let Some(message) = response.first() else {
        return Err("server returned an empty PING response".into());
    };
    validate_pong(
        message.command(),
        message.params().last().map(String::as_str),
        &token,
    )?;
    connection.close().await
}

fn validate_pong(command: &str, last_param: Option<&str>, token: &str) -> Result<(), Failed> {
    if command.eq_ignore_ascii_case("PONG") && last_param == Some(token) {
        Ok(())
    } else {
        Err("server PONG did not contain the PING token".into())
    }
}

#[cfg(test)]
mod tests {
    use super::validate_pong;

    #[test]
    fn wrong_pong_token_fails() {
        let error = validate_pong("PONG", Some("wrong-token"), "papo-ping-417").unwrap_err();
        assert_eq!(
            error.message(),
            Some("server PONG did not contain the PING token")
        );
    }
}
