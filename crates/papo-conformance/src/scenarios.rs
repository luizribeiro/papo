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
        ScenarioSpec {
            name: "account_notify",
            doc: "delivers an authenticated sender account",
            requires: Requirements {
                caps: &[
                    CapRequirement {
                        name: "account-tag",
                        value: None,
                    },
                    CapRequirement {
                        name: "account-notify",
                        value: None,
                    },
                ],
                isupport: &[],
            },
            run: account_notify,
        },
        ScenarioSpec {
            name: "away_notify",
            doc: "notifies channel peers of away state",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "away-notify",
                    value: None,
                }],
                isupport: &[],
            },
            run: away_notify,
        },
        ScenarioSpec {
            name: "setname",
            doc: "notifies channel peers of real-name changes",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "setname",
                    value: None,
                }],
                isupport: &[],
            },
            run: setname,
        },
        ScenarioSpec {
            name: "extended_join",
            doc: "records real names from extended JOIN",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "extended-join",
                    value: None,
                }],
                isupport: &[],
            },
            run: extended_join,
        },
        ScenarioSpec {
            name: "multi_prefix",
            doc: "records every advertised membership prefix",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "multi-prefix",
                    value: None,
                }],
                isupport: &[],
            },
            run: multi_prefix,
        },
        ScenarioSpec {
            name: "userhost_in_names",
            doc: "records usernames and hosts from NAMES",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "userhost-in-names",
                    value: None,
                }],
                isupport: &[],
            },
            run: userhost_in_names,
        },
        ScenarioSpec {
            name: "invite_notify",
            doc: "notifies channel members about invitations",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "invite-notify",
                    value: None,
                }],
                isupport: &[],
            },
            run: invite_notify,
        },
        ScenarioSpec {
            name: "monitor",
            doc: "reports monitored users with extended-monitor enabled",
            requires: Requirements {
                caps: &[
                    CapRequirement {
                        name: "extended-monitor",
                        value: None,
                    },
                    CapRequirement {
                        name: "away-notify",
                        value: None,
                    },
                ],
                isupport: &["MONITOR"],
            },
            run: monitor,
        },
        ScenarioSpec {
            name: "chghost",
            doc: "notifies channel peers of operator host changes",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "chghost",
                    value: None,
                }],
                isupport: &[],
            },
            run: chghost,
        },
        ScenarioSpec {
            name: "batch",
            doc: "delivers channel history in a batch",
            requires: Requirements {
                caps: &[
                    CapRequirement {
                        name: "batch",
                        value: None,
                    },
                    CapRequirement {
                        name: "draft/chathistory",
                        value: None,
                    },
                ],
                isupport: &[],
            },
            run: batch,
        },
        ScenarioSpec {
            name: "draft_chathistory",
            doc: "retrieves a channel history page",
            requires: Requirements {
                caps: &[
                    CapRequirement {
                        name: "batch",
                        value: None,
                    },
                    CapRequirement {
                        name: "draft/chathistory",
                        value: None,
                    },
                ],
                isupport: &[],
            },
            run: draft_chathistory,
        },
        ScenarioSpec {
            name: "draft_multiline",
            doc: "reassembles a client multiline message",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "draft/multiline",
                    value: None,
                }],
                isupport: &[],
            },
            run: draft_multiline,
        },
        ScenarioSpec {
            name: "draft_read_marker",
            doc: "sets and receives a channel read marker",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "draft/read-marker",
                    value: None,
                }],
                isupport: &[],
            },
            run: draft_read_marker,
        },
        ScenarioSpec {
            name: "draft_metadata_2",
            doc: "sets and receives client metadata",
            requires: Requirements {
                caps: &[
                    CapRequirement {
                        name: "batch",
                        value: None,
                    },
                    CapRequirement {
                        name: "draft/metadata-2",
                        value: None,
                    },
                ],
                isupport: &[],
            },
            run: draft_metadata_2,
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

fn account_notify(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let account = context.unique();
        register_account(&context.server, &account, PASSWORD).map_err(Failed::from)?;
        let sender_nick = context.unique();
        let receiver_nick = context.unique();
        let channel = format!("#{}", context.unique());
        let mut sender = context
            .client_with(&sender_nick, Some(Sasl::plain(&account, PASSWORD)))
            .await?;
        join(&context, &mut sender, &channel).await?;
        let mut receiver = context.client(&receiver_nick).await?;
        join(&context, &mut receiver, &channel).await?;
        let text = format!("authenticated message from {sender_nick}");
        sender
            .client
            .privmsg(&channel, &text)
            .await
            .map_err(Failed::from)?;
        context
            .expect(
                &mut receiver.events,
                |event| matches!(event, Event::Message(message) if message.raw_text() == text && message.account() == Some(account.as_str())),
                EXPECT_TIMEOUT,
            )
            .await?;
        close_pair(receiver, sender).await
    })
}

fn away_notify(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let (mut receiver, sender, _channel) = joined_pair(&context).await?;
        let away = "reviewing the IRCv3 specification";
        sender
            .client
            .send(Message::new("AWAY", [away]))
            .await
            .map_err(Failed::from)?;
        context
            .expect(
                &mut receiver.events,
                |event| matches!(event, Event::AwayChanged { nick, message: Some(message) } if nick == &sender.welcome.nick && message == away),
                EXPECT_TIMEOUT,
            )
            .await?;
        close_pair(receiver, sender).await
    })
}

fn setname(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let (mut receiver, sender, _channel) = joined_pair(&context).await?;
        let realname = "Papo Conformance Client";
        sender
            .client
            .send(Message::new("SETNAME", [realname]))
            .await
            .map_err(Failed::from)?;
        context
            .expect(
                &mut receiver.events,
                |event| matches!(event, Event::RealnameChanged { nick, realname: changed } if nick == &sender.welcome.nick && changed == realname),
                EXPECT_TIMEOUT,
            )
            .await?;
        close_pair(receiver, sender).await
    })
}

fn extended_join(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let (receiver, sender, _channel) = joined_pair(&context).await?;
        let observed = receiver.client.state(|state| {
            state
                .user(&sender.welcome.nick)
                .and_then(papo::User::realname)
                == Some("papo conformance client")
        });
        if !observed {
            return Err("extended JOIN did not populate the sender real name".into());
        }
        close_pair(receiver, sender).await
    })
}

fn multi_prefix(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let first_nick = context.unique();
        let second_nick = context.unique();
        let channel = format!("#{}", context.unique());
        let mut first = context.client(&first_nick).await?;
        join(&context, &mut first, &channel).await?;
        first
            .client
            .send(Message::new("MODE", [&channel, "+v", &first_nick]))
            .await
            .map_err(Failed::from)?;
        context
            .expect(
                &mut first.events,
                |event| matches!(event, Event::ModeChanged { target, .. } if target == &channel),
                EXPECT_TIMEOUT,
            )
            .await?;
        let mut second = context.client(&second_nick).await?;
        join(&context, &mut second, &channel).await?;
        let prefixes = second.client.state(|state| {
            state
                .channel(&channel)
                .and_then(|channel| channel.member(&first_nick))
                .map(|member| member.prefixes().to_vec())
        });
        if prefixes.as_deref() != Some(&['@', '+']) {
            return Err(format!(
                "NAMES did not preserve operator and voice prefixes: {prefixes:?}"
            )
            .into());
        }
        close_pair(first, second).await
    })
}

fn userhost_in_names(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let (receiver, sender, _channel) = joined_pair(&context).await?;
        let userhost = receiver.client.state(|state| {
            state.user(&sender.welcome.nick).map(|user| {
                (
                    user.username().map(str::to_owned),
                    user.host().map(str::to_owned),
                )
            })
        });
        if !matches!(userhost, Some((Some(_), Some(_)))) {
            return Err(format!("NAMES did not include a complete userhost: {userhost:?}").into());
        }
        close_pair(receiver, sender).await
    })
}

fn invite_notify(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let (mut observer, inviter, channel) = joined_pair(&context).await?;
        let target_nick = context.unique();
        let target = context.client(&target_nick).await?;
        inviter
            .client
            .send(Message::new("INVITE", [&target_nick, &channel]))
            .await
            .map_err(Failed::from)?;
        context
            .expect(
                &mut observer.events,
                |event| {
                    matches!(event, Event::Raw(message) if {
                        let message = message.as_ref();
                        message.command_is("INVITE")
                            && message.param(0) == Some(target_nick.as_str())
                            && message.param(1) == Some(channel.as_str())
                    })
                },
                EXPECT_TIMEOUT,
            )
            .await?;
        observer.close().await?;
        close_pair(inviter, target).await
    })
}

fn monitor(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let observer_nick = context.unique();
        let target_nick = context.unique();
        let mut observer = context.client(&observer_nick).await?;
        let target = context.client(&target_nick).await?;
        observer
            .client
            .send(Message::new("MONITOR", ["+", &target_nick]))
            .await
            .map_err(Failed::from)?;
        context
            .expect(
                &mut observer.events,
                |event| match event {
                    Event::MonitorOnline { nick, .. } => nick == &target_nick,
                    Event::Raw(message) => {
                        let message = message.as_ref();
                        message.command_is("730") && message.param(1) == Some(target_nick.as_str())
                    }
                    _ => false,
                },
                EXPECT_TIMEOUT,
            )
            .await
            .map_err(|_| Failed::from("timed out waiting for MONITOR presence"))?;
        close_pair(observer, target).await
    })
}

fn chghost(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let oper = match required_environment("PAPO_TEST_OPER") {
            Ok(oper) => oper,
            Err(ignored) => return Ok(ignored),
        };
        let Some((oper_name, password)) = oper.split_once(':') else {
            return Err("PAPO_TEST_OPER must contain name:password".into());
        };
        let (mut observer, operator, _channel) = joined_pair(&context).await?;
        operator
            .client
            .send(Message::new("OPER", [oper_name, password]))
            .await
            .map_err(Failed::from)?;
        operator
            .client
            .send(Message::new(
                "CHGHOST",
                [&observer.welcome.nick, "conformance.example"],
            ))
            .await
            .map_err(Failed::from)?;
        context
            .expect(
                &mut observer.events,
                |event| matches!(event, Event::HostChanged { nick, host, .. } if nick == &observer.welcome.nick && host == "conformance.example"),
                EXPECT_TIMEOUT,
            )
            .await?;
        close_pair(observer, operator).await
    })
}

fn batch(context: Scenario) -> ScenarioFuture {
    Box::pin(history_page(context))
}

fn draft_chathistory(context: Scenario) -> ScenarioFuture {
    Box::pin(history_page(context))
}

async fn history_page(context: Scenario) -> ScenarioResult {
    let writer_nick = context.unique();
    let reader_nick = context.unique();
    let channel = format!("#{}", context.unique());
    let text = format!("history from {writer_nick}");
    let mut writer = context.client(&writer_nick).await?;
    join(&context, &mut writer, &channel).await?;
    writer
        .client
        .privmsg(&channel, &text)
        .await
        .map_err(Failed::from)?;
    let mut reader = context
        .client_caps(&reader_nick, &["draft/chathistory"])
        .await?;
    join(&context, &mut reader, &channel).await?;
    let response = reader
        .client
        .raw(Message::new("CHATHISTORY", ["LATEST", &channel, "*", "10"]))
        .await
        .map_err(Failed::from)?;
    let Some(history) = response.history else {
        return Err("CHATHISTORY response did not contain a history batch".into());
    };
    if !history
        .messages
        .iter()
        .any(|message| message.as_ref().param(1) == Some(text.as_str()))
    {
        return Err("history batch did not contain the earlier message".into());
    }
    close_pair(writer, reader).await
}

fn draft_multiline(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let (receiver, mut sender, channel) = joined_pair(&context).await?;
        let reference = context.unique().replace('_', "-");
        sender
            .client
            .send(Message::new(
                "BATCH",
                [
                    format!("+{reference}"),
                    "draft/multiline".into(),
                    channel.clone(),
                ],
            ))
            .await
            .map_err(Failed::from)?;
        sender
            .client
            .send(Message::privmsg(&channel, "first ").with_tag("batch", reference.clone()))
            .await
            .map_err(Failed::from)?;
        sender
            .client
            .send(
                Message::privmsg(&channel, "second")
                    .with_tag("batch", reference.clone())
                    .with_tag("draft/multiline-concat", ""),
            )
            .await
            .map_err(Failed::from)?;
        sender
            .client
            .send(Message::new("BATCH", [format!("-{reference}")]))
            .await
            .map_err(Failed::from)?;
        context
            .expect(
                &mut sender.events,
                |event| matches!(event, Event::Message(message) if message.raw_text() == "first second"),
                EXPECT_TIMEOUT,
            )
            .await?;
        close_pair(receiver, sender).await
    })
}

fn draft_read_marker(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let nick = context.unique();
        let channel = format!("#{}", context.unique());
        let mut connection = context.client_caps(&nick, &["draft/read-marker"]).await?;
        join(&context, &mut connection, &channel).await?;
        connection
            .client
            .send(Message::new(
                "MARKREAD",
                [&channel, "timestamp=2025-10-15T11:55:42.123Z"],
            ))
            .await
            .map_err(Failed::from)?;
        context
            .expect(
                &mut connection.events,
                |event| matches!(event, Event::ReadMarker(marker) if marker.target == channel && marker.time.is_some()),
                EXPECT_TIMEOUT,
            )
            .await?;
        connection.close().await
    })
}

fn draft_metadata_2(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let nick = context.unique();
        let key = "example.org/papo-conformance";
        let value = context.unique();
        let mut connection = context.client(&nick).await?;
        let response = connection
            .client
            .raw(Message::new("METADATA", [&nick, "SET", key, &value]))
            .await
            .map_err(Failed::from)?;
        if let Some(error) = response.error() {
            return Err(format!("server rejected METADATA SET: {error:?}").into());
        }
        context
            .expect(
                &mut connection.events,
                |event| matches!(event, Event::MetadataChanged { target, key: changed_key, value: Some(changed_value), .. } if target == &nick && changed_key == key && changed_value == &value),
                EXPECT_TIMEOUT,
            )
            .await?;
        connection.close().await
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
