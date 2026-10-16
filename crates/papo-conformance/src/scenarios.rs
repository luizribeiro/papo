use libtest_mimic::Failed;
use papo::Event;
use papo_conformance_macros::scenario;
use papo_core::Sasl;
use papo_proto::Message;
use papo_test_server::register_account;

use super::{Scenario, ScenarioResult, ScenarioSpec, required_environment};
use support::{EXPECT_TIMEOUT, PASSWORD, close_pair, join, joined_pair};

mod auth;
mod connection;
mod history;
mod membership;
mod messaging;
mod metadata;
mod requests;
mod support;

pub(super) fn all() -> Vec<ScenarioSpec> {
    let mut scenarios = inventory::iter::<ScenarioSpec>
        .into_iter()
        .copied()
        .collect::<Vec<_>>();
    scenarios.sort_unstable_by_key(|scenario| scenario.name);
    scenarios
}

#[cfg(any())]
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

#[cfg(test)]
mod tests {
    use super::all;

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
}
