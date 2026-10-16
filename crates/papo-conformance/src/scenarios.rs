use libtest_mimic::Failed;
use papo::Event;
use papo_conformance_macros::scenario;
use papo_core::Sasl;
use papo_proto::{Message, MessageRef, Source, StandardReplyKind, Typing};
use papo_test_server::register_account;

use super::{
    CapRequirement, Requirements, Scenario, ScenarioResult, ScenarioSpec, required_environment,
};
use support::{EXPECT_TIMEOUT, PASSWORD, close_pair, join, joined_pair};

mod support;

pub(super) fn all() -> Vec<ScenarioSpec> {
    let mut scenarios = inventory::iter::<ScenarioSpec>
        .into_iter()
        .copied()
        .collect::<Vec<_>>();
    scenarios.sort_unstable_by_key(|scenario| scenario.name);
    scenarios
}

#[allow(dead_code, clippy::too_many_lines)]
const fn scenario_table() -> &'static [ScenarioSpec] {
    &[
        ScenarioSpec {
            name: "registration",
            doc: "registers with the requested nickname",
            requires: Requirements {
                caps: &[],
                isupport: &[],
            },
            run: __papo_scenario_registration,
        },
        ScenarioSpec {
            name: "tls",
            doc: "registers over TLS with the configured test CA",
            requires: Requirements {
                caps: &[],
                isupport: &[],
            },
            run: __papo_scenario_tls,
        },
        ScenarioSpec {
            name: "websocket",
            doc: "registers over a WebSocket transport",
            requires: Requirements {
                caps: &[],
                isupport: &[],
            },
            run: __papo_scenario_websocket,
        },
        ScenarioSpec {
            name: "websocket_tls",
            doc: "registers over a TLS WebSocket transport",
            requires: Requirements {
                caps: &[],
                isupport: &[],
            },
            run: __papo_scenario_websocket_tls,
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
            run: __papo_scenario_sasl_plain,
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
            run: __papo_scenario_sasl_scram_sha_256,
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
            run: __papo_scenario_ping,
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
            run: __papo_scenario_echo_message,
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
            run: __papo_scenario_labeled_response,
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
            run: __papo_scenario_server_time,
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
            run: __papo_scenario_message_tags,
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
            run: __papo_scenario_account_notify,
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
            run: __papo_scenario_away_notify,
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
            run: __papo_scenario_setname,
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
            run: __papo_scenario_extended_join,
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
            run: __papo_scenario_multi_prefix,
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
            run: __papo_scenario_userhost_in_names,
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
            run: __papo_scenario_invite_notify,
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
            run: __papo_scenario_monitor,
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
            run: __papo_scenario_chghost,
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
            run: __papo_scenario_batch,
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
            run: __papo_scenario_draft_chathistory,
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
            run: __papo_scenario_draft_multiline,
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
            run: __papo_scenario_draft_read_marker,
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
            run: __papo_scenario_draft_metadata_2,
        },
        ScenarioSpec {
            name: "draft_channel_rename",
            doc: "renames a joined channel",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "draft/channel-rename",
                    value: None,
                }],
                isupport: &[],
            },
            run: __papo_scenario_draft_channel_rename,
        },
        ScenarioSpec {
            name: "draft_account_registration",
            doc: "registers and authenticates a new account",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "draft/account-registration",
                    value: None,
                }],
                isupport: &[],
            },
            run: __papo_scenario_draft_account_registration,
        },
        ScenarioSpec {
            name: "draft_message_redaction",
            doc: "redacts a delivered channel message",
            requires: Requirements {
                caps: &[CapRequirement {
                    name: "draft/message-redaction",
                    value: None,
                }],
                isupport: &[],
            },
            run: __papo_scenario_draft_message_redaction,
        },
        ScenarioSpec {
            name: "standard_replies",
            doc: "rejects an invalid real name with a standard failure",
            requires: Requirements {
                caps: &[
                    CapRequirement {
                        name: "standard-replies",
                        value: None,
                    },
                    CapRequirement {
                        name: "setname",
                        value: None,
                    },
                ],
                isupport: &[],
            },
            run: __papo_scenario_standard_replies,
        },
        ScenarioSpec {
            name: "bot_mode",
            doc: "marks a client with the advertised bot mode",
            requires: Requirements {
                caps: &[],
                isupport: &["BOT"],
            },
            run: __papo_scenario_bot_mode,
        },
        ScenarioSpec {
            name: "utf8only",
            doc: "delivers UTF-8 channel text without replacement",
            requires: Requirements {
                caps: &[],
                isupport: &["UTF8ONLY"],
            },
            run: __papo_scenario_utf8only,
        },
        ScenarioSpec {
            name: "whox",
            doc: "parses an extended WHO reply",
            requires: Requirements {
                caps: &[],
                isupport: &["WHOX"],
            },
            run: __papo_scenario_whox,
        },
    ]
}

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

async fn validate_registration(connection: super::RegisteredClient, nick: &str) -> ScenarioResult {
    if connection.welcome.nick != nick {
        return Err("welcome nickname differs from the requested nickname".into());
    }
    connection.close().await
}

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

/// echoes a channel PRIVMSG to its sender
#[scenario(requires(caps = ["echo-message"]))]
async fn echo_message(context: Scenario) -> ScenarioResult {
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
}

/// correlates a labelled WHOIS response
#[scenario(requires(caps = ["labeled-response"]))]
async fn labeled_response(context: Scenario) -> ScenarioResult {
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
}

/// delivers a channel message with server time
#[scenario(requires(caps = ["server-time"]))]
async fn server_time(context: Scenario) -> ScenarioResult {
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
}

/// delivers a client-only typing tag
#[scenario(requires(caps = ["message-tags"]))]
async fn message_tags(context: Scenario) -> ScenarioResult {
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
}

/// delivers an authenticated sender account
#[scenario(requires(caps = ["account-tag", "account-notify"]))]
async fn account_notify(context: Scenario) -> ScenarioResult {
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
}

/// notifies channel peers of away state
#[scenario(requires(caps = ["away-notify"]))]
async fn away_notify(context: Scenario) -> ScenarioResult {
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
}

/// notifies channel peers of real-name changes
#[scenario(requires(caps = ["setname"]))]
async fn setname(context: Scenario) -> ScenarioResult {
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
}

/// records real names from extended JOIN
#[scenario(requires(caps = ["extended-join"]))]
async fn extended_join(context: Scenario) -> ScenarioResult {
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
}

/// records every advertised membership prefix
#[scenario(requires(caps = ["multi-prefix"]))]
async fn multi_prefix(context: Scenario) -> ScenarioResult {
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
        return Err(
            format!("NAMES did not preserve operator and voice prefixes: {prefixes:?}").into(),
        );
    }
    close_pair(first, second).await
}

/// records usernames and hosts from NAMES
#[scenario(requires(caps = ["userhost-in-names"]))]
async fn userhost_in_names(context: Scenario) -> ScenarioResult {
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
}

/// notifies channel members about invitations
#[scenario(requires(caps = ["invite-notify"]))]
async fn invite_notify(context: Scenario) -> ScenarioResult {
    let (inviter, mut observer, channel) = joined_pair(&context).await?;
    let observer_nick = observer.welcome.nick.clone();
    if !observer
        .client
        .state(|state| state.has_cap("invite-notify"))
    {
        return Err("invite-notify observer did not negotiate the capability".into());
    }
    inviter
        .client
        .send(Message::new("MODE", [&channel, "+o", &observer_nick]))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut observer.events,
            |event| matches!(event, Event::ModeChanged { target, .. } if target == &channel),
            EXPECT_TIMEOUT,
        )
        .await?;
    let observer_is_op = observer.client.state(|state| {
        state
            .channel(&channel)
            .and_then(|channel| channel.member(&observer_nick))
            .is_some_and(papo_core::Member::is_op)
    });
    if !observer_is_op {
        return Err("invite-notify observer is not a channel operator".into());
    }
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
}

/// reports monitored users with extended-monitor enabled
#[scenario(requires(
    caps = ["extended-monitor", "away-notify"],
    isupport = ["MONITOR"]
))]
async fn monitor(context: Scenario) -> ScenarioResult {
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
            |event| matches!(event, Event::MonitorOnline { nick, .. } if nick == &target_nick),
            EXPECT_TIMEOUT,
        )
        .await
        .map_err(|_| Failed::from("timed out waiting for MONITOR presence"))?;
    close_pair(observer, target).await
}

/// notifies channel peers of operator host changes
#[scenario(requires(caps = ["chghost"]))]
async fn chghost(context: Scenario) -> ScenarioResult {
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
}

/// delivers channel history in a batch
#[scenario(requires(caps = ["batch", "draft/chathistory"]))]
async fn batch(context: Scenario) -> ScenarioResult {
    history_page(context).await
}

/// retrieves a channel history page
#[scenario(requires(caps = ["batch", "draft/chathistory"]))]
async fn draft_chathistory(context: Scenario) -> ScenarioResult {
    history_page(context).await
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

/// reassembles a client multiline message
#[scenario(requires(caps = ["draft/multiline"]))]
async fn draft_multiline(context: Scenario) -> ScenarioResult {
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
}

/// sets and receives a channel read marker
#[scenario(requires(caps = ["draft/read-marker"]))]
async fn draft_read_marker(context: Scenario) -> ScenarioResult {
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
}

/// sets and receives client metadata
#[scenario(requires(caps = ["batch", "draft/metadata-2"]))]
async fn draft_metadata_2(context: Scenario) -> ScenarioResult {
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
}

/// renames a joined channel
#[scenario(requires(caps = ["draft/channel-rename"]))]
async fn draft_channel_rename(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let old = format!("#{}", context.unique());
    let new = format!("#{}", context.unique());
    let mut connection = context
        .client_caps(&nick, &["draft/channel-rename"])
        .await?;
    join(&context, &mut connection, &old).await?;
    let response = connection
        .client
        .raw(Message::new("RENAME", [&old, &new, "conformance rename"]))
        .await
        .map_err(Failed::from)?;
    if let Some(error) = response.error() {
        return Err(format!("server rejected RENAME: {error:?}").into());
    }
    context
            .expect(
                &mut connection.events,
                |event| matches!(event, Event::ChannelRenamed { old: renamed, new: replacement, .. } if renamed == &old && replacement == &new),
                EXPECT_TIMEOUT,
            )
            .await?;
    connection.close().await
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
        .send(Message::new("REGISTER", [&account, "*", PASSWORD]))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| match event {
                Event::AccountRegistered {
                    account: registered,
                    logged_in: true,
                } => registered == &account,
                Event::Raw(message) => {
                    let message = message.as_ref();
                    message.command_is("REGISTER")
                        && message.param(0) == Some("SUCCESS")
                        && message.param(1) == Some(account.as_str())
                }
                _ => false,
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    connection.close().await
}

/// redacts a delivered channel message
#[scenario(requires(caps = ["draft/message-redaction"]))]
async fn draft_message_redaction(context: Scenario) -> ScenarioResult {
    let sender_nick = context.unique();
    let receiver_nick = context.unique();
    let channel = format!("#{}", context.unique());
    let mut sender = context
        .client_caps(&sender_nick, &["draft/message-redaction"])
        .await?;
    join(&context, &mut sender, &channel).await?;
    let mut receiver = context
        .client_caps(&receiver_nick, &["draft/message-redaction"])
        .await?;
    join(&context, &mut receiver, &channel).await?;
    let response = sender
        .client
        .raw(Message::privmsg(&channel, "message to redact"))
        .await
        .map_err(Failed::from)?;
    let Some(sent) = response.sent() else {
        return Err("echoed message did not include a message identifier".into());
    };
    sender
        .client
        .send(Message::new(
            "REDACT",
            [&channel, &sent.msgid, "conformance redaction"],
        ))
        .await
        .map_err(Failed::from)?;
    context
            .expect(
                &mut receiver.events,
                |event| matches!(event, Event::Redacted(redacted) if redacted.target == channel && redacted.msgid == sent.msgid),
                EXPECT_TIMEOUT,
            )
            .await?;
    close_pair(sender, receiver).await
}

/// rejects an invalid real name with a standard failure
#[scenario(requires(caps = ["standard-replies", "setname"]))]
async fn standard_replies(context: Scenario) -> ScenarioResult {
    if std::env::var("PAPO_TEST_SERVER").is_ok_and(|server| server != "ergo") {
        return Ok(libtest_mimic::Completion::ignored_with(
            "portable standard-replies failure is not set up",
        ));
    }
    let nick = context.unique();
    let mut connection = context.client(&nick).await?;
    connection
        .client
        .send(Message::new("SETNAME", [""]))
        .await
        .map_err(Failed::from)?;
    context
            .expect(
                &mut connection.events,
                |event| {
                    matches!(event, Event::StandardReply(reply) if reply.kind == StandardReplyKind::Fail && reply.command == "SETNAME" && reply.code == "INVALID_REALNAME")
                },
                EXPECT_TIMEOUT,
            )
            .await?;
    connection.close().await
}

/// marks a client with the advertised bot mode
#[scenario(requires(isupport = ["BOT"]))]
async fn bot_mode(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let mut connection = context.client(&nick).await?;
    let registered = connection.welcome.nick.clone();
    connection
        .client
        .send(Message::new("MODE", [&registered, "+B"]))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| matches!(event, Event::ModeChanged { target, .. } if target == &registered),
            EXPECT_TIMEOUT,
        )
        .await?;
    let whois = connection
        .client
        .whois(&registered)
        .await
        .map_err(Failed::from)?;
    if !whois.is_bot {
        return Err("WHOIS did not report the advertised bot mode".into());
    }
    connection.close().await
}

/// delivers UTF-8 channel text without replacement
#[scenario(requires(isupport = ["UTF8ONLY"]))]
async fn utf8only(context: Scenario) -> ScenarioResult {
    let (mut receiver, sender, channel) = joined_pair(&context).await?;
    let text = "Olá, IRC 世界";
    sender
        .client
        .privmsg(&channel, text)
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut receiver.events,
            |event| matches!(event, Event::Message(message) if message.raw_text() == text),
            EXPECT_TIMEOUT,
        )
        .await?;
    close_pair(receiver, sender).await
}

/// parses an extended WHO reply
#[scenario(requires(isupport = ["WHOX"]))]
async fn whox(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let connection = context.client(&nick).await?;
    let entries = connection
        .client
        .who(&connection.welcome.nick)
        .await
        .map_err(Failed::from)?;
    let complete = entries.iter().any(|entry| {
        entry.nick == connection.welcome.nick
            && !entry.user.is_empty()
            && !entry.host.is_empty()
            && !entry.realname.is_empty()
    });
    if !complete {
        return Err("WHOX reply did not contain the registered user's identity".into());
    }
    connection.close().await
}

#[cfg(test)]
mod tests {
    use papo_proto::MessageRef;

    use super::{all, validate_account_reply, validate_pong};

    #[test]
    fn registered_scenarios_are_complete_and_sorted() {
        let names = all()
            .into_iter()
            .map(|scenario| scenario.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "account_notify",
                "away_notify",
                "batch",
                "bot_mode",
                "chghost",
                "draft_account_registration",
                "draft_channel_rename",
                "draft_chathistory",
                "draft_message_redaction",
                "draft_metadata_2",
                "draft_multiline",
                "draft_read_marker",
                "echo_message",
                "extended_join",
                "invite_notify",
                "labeled_response",
                "message_tags",
                "monitor",
                "multi_prefix",
                "ping",
                "registration",
                "sasl_plain",
                "sasl_scram_sha_256",
                "server_time",
                "setname",
                "standard_replies",
                "tls",
                "userhost_in_names",
                "utf8only",
                "websocket",
                "websocket_tls",
                "whox",
            ]
        );
    }

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
