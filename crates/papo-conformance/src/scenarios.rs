use super::ScenarioSpec;

mod auth;
mod connection;
mod history;
mod membership;
mod messaging;
mod metadata;
mod presence;
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
