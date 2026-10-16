use std::time::Duration;

use libtest_mimic::Failed;
use papo::Event;
use papo_core::Sasl;
use papo_proto::{Message, MessageRef, Source, Typing};
use papo_test_server::register_account;

use super::{
    CapRequirement, Requirements, Scenario, ScenarioFuture, ScenarioResult, ScenarioSpec,
    required_environment,
};

const EXPECT_TIMEOUT: Duration = Duration::from_secs(10);
const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

// Keeping each requirement beside its scenario makes support-matrix gating auditable.
#[allow(clippy::too_many_lines)]
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
            name: "tls",
            doc: "registers over TLS with the configured test CA",
            requires: Requirements {
                caps: &[],
                isupport: &[],
            },
            run: tls,
        },
        ScenarioSpec {
            name: "websocket",
            doc: "registers over a WebSocket transport",
            requires: Requirements {
                caps: &[],
                isupport: &[],
            },
            run: websocket,
        },
        ScenarioSpec {
            name: "websocket_tls",
            doc: "registers over a TLS WebSocket transport",
            requires: Requirements {
                caps: &[],
                isupport: &[],
            },
            run: websocket_tls,
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
        ScenarioSpec {
            name: "ping",
            doc: "answers a client PING with the matching PONG",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "labeled-response",
                    value: None,
                }],
                isupport: &[],
            },
            run: ping,
        },
        ScenarioSpec {
            name: "echo_message",
            doc: "echoes a channel PRIVMSG to its sender",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "echo-message",
                    value: None,
                }],
                isupport: &[],
            },
            run: echo_message,
        },
        ScenarioSpec {
            name: "labeled_response",
            doc: "correlates a labelled WHOIS response",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "labeled-response",
                    value: None,
                }],
                isupport: &[],
            },
            run: labeled_response,
        },
        ScenarioSpec {
            name: "server_time",
            doc: "delivers a channel message with server time",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "server-time",
                    value: None,
                }],
                isupport: &[],
            },
            run: server_time,
        },
        ScenarioSpec {
            name: "message_tags",
            doc: "delivers a client-only typing tag",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "message-tags",
                    value: None,
                }],
                isupport: &[],
            },
            run: message_tags,
        },
    ]
}

fn registration(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let nick = context.unique();
        let connection = context.client(&nick).await?;
        validate_registration(connection, &nick).await
    })
}

fn tls(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let address = format!("ircs://{}", context.server.tls_addr());
        register_over(context, &address).await
    })
}

fn websocket(context: Scenario) -> ScenarioFuture {
    register_over_environment(context, "PAPO_TEST_WS")
}

fn websocket_tls(context: Scenario) -> ScenarioFuture {
    register_over_environment(context, "PAPO_TEST_WSS")
}

fn register_over_environment(context: Scenario, variable: &'static str) -> ScenarioFuture {
    Box::pin(async move {
        let address = match required_environment(variable) {
            Ok(address) => address,
            Err(ignored) => return Ok(ignored),
        };
        register_over(context, &address).await
    })
}

async fn register_over(context: Scenario, address: &str) -> ScenarioResult {
    let nick = context.unique();
    let connection = context.client_at(&nick, address).await?;
    validate_registration(connection, &nick).await
}

async fn validate_registration(connection: super::RegisteredClient, nick: &str) -> ScenarioResult {
    if connection.welcome.nick != nick {
        return Err("welcome nickname differs from the requested nickname".into());
    }
    connection.close().await
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
    validate_account_reply(&message.as_ref(), &account)?;
    connection.close().await
}

fn validate_account_reply(message: &MessageRef<'_>, account: &str) -> Result<(), Failed> {
    if (message.command_is("900") || message.command_is("330")) && message.param(2) == Some(account)
    {
        Ok(())
    } else {
        Err("server account reply did not contain the authenticated account".into())
    }
}

fn ping(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
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
    })
}

fn validate_pong(message: &MessageRef<'_>, token: &str) -> Result<(), Failed> {
    if message.command_is("PONG") && message.params().last() == Some(token) {
        Ok(())
    } else {
        Err("server PONG did not contain the PING token".into())
    }
}

fn echo_message(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let nick = context.unique();
        let channel = format!("#{}", context.unique());
        let text = format!("echo from {}", context.unique());
        let mut connection = context.client(&nick).await?;
        let registered_nick = connection.welcome.nick.clone();
        connection
            .client
            .send(Message::join(&channel))
            .await
            .map_err(Failed::from)?;
        context
            .expect(
                &mut connection.events,
                |event| {
                    matches!(
                        event,
                        Event::Joined {
                            channel: joined,
                            nick: joined_nick,
                        } if joined == &channel && joined_nick == &registered_nick
                    )
                },
                EXPECT_TIMEOUT,
            )
            .await?;
        connection
            .client
            .privmsg(&channel, &text)
            .await
            .map_err(Failed::from)?;
        context
            .expect(
                &mut connection.events,
                |event| {
                    matches!(event, Event::Message(message) if {
                        let message = message.as_ref();
                        message.command_is("PRIVMSG")
                            && message.param(0) == Some(channel.as_str())
                            && message.param(1) == Some(text.as_str())
                            && matches!(message.source(), Some(Source::User { nick, .. }) if nick == registered_nick)
                    })
                },
                EXPECT_TIMEOUT,
            )
            .await?;
        connection.close().await
    })
}

async fn joined_pair(
    context: &Scenario,
) -> Result<(super::RegisteredClient, super::RegisteredClient, String), Failed> {
    let first_nick = context.unique();
    let second_nick = context.unique();
    let channel = format!("#{}", context.unique());
    let mut first = context.client(&first_nick).await?;
    join(context, &mut first, &channel).await?;
    let mut second = context.client(&second_nick).await?;
    join(context, &mut second, &channel).await?;
    context
        .expect(
            &mut first.events,
            |event| {
                matches!(event, Event::Joined { channel: joined, nick } if joined == &channel && nick == &second_nick)
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    Ok((first, second, channel))
}

async fn join(
    context: &Scenario,
    connection: &mut super::RegisteredClient,
    channel: &str,
) -> Result<(), Failed> {
    connection
        .client
        .send(Message::join(channel))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| matches!(event, Event::ChannelSynced { channel: joined } if joined == channel),
            EXPECT_TIMEOUT,
        )
        .await?;
    Ok(())
}

async fn close_pair(
    first: super::RegisteredClient,
    second: super::RegisteredClient,
) -> ScenarioResult {
    first.close().await?;
    second.close().await
}

fn labeled_response(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let nick = context.unique();
        let connection = context.client(&nick).await?;
        let whois = connection
            .client
            .whois(&connection.welcome.nick)
            .await
            .map_err(Failed::from)?;
        if whois.nick != connection.welcome.nick {
            return Err("labelled WHOIS resolved for the wrong nickname".into());
        }
        connection.close().await
    })
}

fn server_time(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let (mut receiver, sender, channel) = joined_pair(&context).await?;
        let text = format!("timestamped message from {}", sender.welcome.nick);
        sender
            .client
            .privmsg(&channel, &text)
            .await
            .map_err(Failed::from)?;
        context
            .expect(
                &mut receiver.events,
                |event| {
                    matches!(event, Event::Message(message) if message.raw_text() == text && message.as_ref().tag("time").is_some())
                },
                EXPECT_TIMEOUT,
            )
            .await?;
        close_pair(receiver, sender).await
    })
}

fn message_tags(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let (mut receiver, sender, channel) = joined_pair(&context).await?;
        sender
            .client
            .send(Message::new("TAGMSG", [&channel]).with_tag("+typing", "active"))
            .await
            .map_err(Failed::from)?;
        context
            .expect(
                &mut receiver.events,
                |event| {
                    matches!(event, Event::Typing(message) if message.target() == channel && message.typing() == Some(Typing::Active))
                },
                EXPECT_TIMEOUT,
            )
            .await?;
        close_pair(receiver, sender).await
    })
}

#[cfg(test)]
mod tests {
    use papo_proto::MessageRef;

    use super::{validate_account_reply, validate_pong};

    #[test]
    fn wrong_account_in_logged_in_reply_fails() {
        let message = MessageRef::parse(
            ":ergo.test 900 papo papo!~u@host mallory :You are now logged in as mallory",
        )
        .unwrap();
        let error = validate_account_reply(&message, "alice").unwrap_err();
        assert_eq!(
            error.message(),
            Some("server account reply did not contain the authenticated account")
        );
    }

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
