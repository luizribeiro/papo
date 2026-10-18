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
    // WASI command linking does not guarantee that inventory constructors run.
    #[cfg(all(target_os = "wasi", not(target_env = "p1")))]
    let mut scenarios = vec![
        auth::__papo_scenario_spec_sasl_plain,
        auth::__papo_scenario_spec_sasl_scram_sha_256,
        auth::__papo_scenario_spec_sasl_failure,
        auth::__papo_scenario_spec_sasl_protocol_errors,
        auth::__papo_scenario_spec_draft_account_registration,
        auth::__papo_scenario_spec_account_registration_error,
        connection::__papo_scenario_spec_registration,
        connection::__papo_scenario_spec_capability_negotiation,
        connection::__papo_scenario_spec_isupport,
        connection::__papo_scenario_spec_server_information,
        connection::__papo_scenario_spec_tls,
        connection::__papo_scenario_spec_tls_certificate_rejected,
        connection::__papo_scenario_spec_websocket,
        connection::__papo_scenario_spec_websocket_tls,
        connection::__papo_scenario_spec_ping,
        history::__papo_scenario_spec_draft_chathistory,
        history::__papo_scenario_spec_draft_read_marker,
        membership::__papo_scenario_spec_extended_join,
        membership::__papo_scenario_spec_userhost_in_names,
        membership::__papo_scenario_spec_whox,
        membership::__papo_scenario_spec_multi_prefix,
        membership::__papo_scenario_spec_invite_notify,
        messaging::__papo_scenario_spec_echo_message,
        messaging::__papo_scenario_spec_server_time,
        messaging::__papo_scenario_spec_message_tags,
        messaging::__papo_scenario_spec_draft_multiline,
        messaging::__papo_scenario_spec_utf8only,
        messaging::__papo_scenario_spec_draft_message_redaction,
        metadata::__papo_scenario_spec_draft_metadata_2,
        metadata::__papo_scenario_spec_metadata_subscriptions,
        metadata::__papo_scenario_spec_channel_metadata,
        metadata::__papo_scenario_spec_metadata_error,
        metadata::__papo_scenario_spec_channel_rename_error,
        metadata::__papo_scenario_spec_draft_channel_rename,
        presence::__papo_scenario_spec_account_notify,
        presence::__papo_scenario_spec_away_notify,
        presence::__papo_scenario_spec_setname,
        presence::__papo_scenario_spec_chghost,
        presence::__papo_scenario_spec_monitor,
        presence::__papo_scenario_spec_bot_mode,
        requests::__papo_scenario_spec_labeled_response,
        requests::__papo_scenario_spec_batch,
        requests::__papo_scenario_spec_standard_replies,
    ];
    #[cfg(not(all(target_os = "wasi", not(target_env = "p1"))))]
    let mut scenarios = inventory::iter::<ScenarioSpec>
        .into_iter()
        .copied()
        .collect::<Vec<_>>();
    scenarios.sort_unstable_by_key(|scenario| scenario.name);
    scenarios
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
                "account_registration_error",
                "away_notify",
                "batch",
                "bot_mode",
                "capability_negotiation",
                "channel_metadata",
                "channel_rename_error",
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
                "isupport",
                "labeled_response",
                "message_tags",
                "metadata_error",
                "metadata_subscriptions",
                "monitor",
                "multi_prefix",
                "ping",
                "registration",
                "sasl_failure",
                "sasl_plain",
                "sasl_protocol_errors",
                "sasl_scram_sha_256",
                "server_information",
                "server_time",
                "setname",
                "standard_replies",
                "tls",
                "tls_certificate_rejected",
                "userhost_in_names",
                "utf8only",
                "websocket",
                "websocket_tls",
                "whox",
            ]
        );
    }
}
