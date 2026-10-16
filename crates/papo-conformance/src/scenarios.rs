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
