use libtest_mimic::Failed;
use papo_conformance_macros::scenario;
use papo_proto::{Message, MessageRef};

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

/// answers a client PING with the matching PONG
#[scenario(requires(caps = ["labeled-response"]))]
async fn ping(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let connection = context.client(&nick).await?;
    let token = context.unique();
    let response = connection
        .client
        .raw(Message::new("PING", [&token]))
        .await
        .map_err(Failed::from)?;
    let Some(message) = response.lines.first() else {
        return Err("server returned an empty PING response".into());
    };
    validate_pong(&message.as_ref(), &token)?;
    connection.close().await
}

fn validate_pong(message: &MessageRef<'_>, token: &str) -> Result<(), Failed> {
    if message.command_is("PONG") && message.params().last() == Some(token) {
        Ok(())
    } else {
        Err("server PONG did not contain the PING token".into())
    }
}

#[cfg(test)]
mod tests {
    use papo_proto::MessageRef;

    use super::validate_pong;

    #[test]
    fn wrong_pong_token_fails() {
        let message = MessageRef::parse(":ergo.test PONG ergo.test :wrong-token").unwrap();
        let error = validate_pong(&message, "papo-ping-417").unwrap_err();
        assert_eq!(
            error.message(),
            Some("server PONG did not contain the PING token")
        );
    }
}
