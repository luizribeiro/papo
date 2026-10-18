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
        auth::__papo_scenario_spec_persistence,
        connection::__papo_scenario_spec_registration,
        connection::__papo_scenario_spec_cap_notify_new,
        connection::__papo_scenario_spec_capability_negotiation,
        connection::__papo_scenario_spec_isupport,
        connection::__papo_scenario_spec_extended_isupport,
        connection::__papo_scenario_spec_server_information,
        connection::__papo_scenario_spec_bouncer_networks,
        connection::__papo_scenario_spec_sts,
        connection::__papo_scenario_spec_tls,
        connection::__papo_scenario_spec_tls_certificate_rejected,
        connection::__papo_scenario_spec_websocket,
        connection::__papo_scenario_spec_websocket_tls,
        connection::__papo_scenario_spec_webirc,
        connection::__papo_scenario_spec_ping,
        history::__papo_scenario_spec_draft_chathistory,
        history::__papo_scenario_spec_chathistory_paging,
        history::__papo_scenario_spec_chathistory_errors,
        history::__papo_scenario_spec_event_playback,
        history::__papo_scenario_spec_draft_read_marker,
        history::__papo_scenario_spec_read_marker_on_join,
        membership::__papo_scenario_spec_extended_join,
        membership::__papo_scenario_spec_channel_modes_and_join_refusals,
        membership::__papo_scenario_spec_userhost_in_names,
        membership::__papo_scenario_spec_multiple_joins,
        membership::__papo_scenario_spec_whox,
        membership::__papo_scenario_spec_multi_prefix,
        membership::__papo_scenario_spec_invite_notify,
        membership::__papo_scenario_spec_invite_rules,
        membership::__papo_scenario_spec_kick,
        membership::__papo_scenario_spec_kick_rules,
        membership::__papo_scenario_spec_nick_change_and_collision,
        membership::__papo_scenario_spec_part,
        membership::__papo_scenario_spec_part_multiple_and_refusal,
        membership::__papo_scenario_spec_topic,
        messaging::__papo_scenario_spec_echo_message,
        messaging::__papo_scenario_spec_ctcp,
        messaging::__papo_scenario_spec_server_time,
        messaging::__papo_scenario_spec_message_tags,
        messaging::__papo_scenario_spec_direct_messages,
        messaging::__papo_scenario_spec_status_message,
        messaging::__papo_scenario_spec_replies_and_reactions,
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
        presence::__papo_scenario_spec_account_tag,
        presence::__papo_scenario_spec_away_notify,
        presence::__papo_scenario_spec_setname,
        presence::__papo_scenario_spec_chghost,
        presence::__papo_scenario_spec_monitor,
        presence::__papo_scenario_spec_bot_mode,
        presence::__papo_scenario_spec_who_and_whois,
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
                "account_tag",
                "away_notify",
                "batch",
                "bot_mode",
                "bouncer_networks",
                "cap_notify_new",
                "capability_negotiation",
                "channel_metadata",
                "channel_modes_and_join_refusals",
                "channel_rename_error",
                "chathistory_errors",
                "chathistory_paging",
                "chghost",
                "ctcp",
                "direct_messages",
                "draft_account_registration",
                "draft_channel_rename",
                "draft_chathistory",
                "draft_message_redaction",
                "draft_metadata_2",
                "draft_multiline",
                "draft_read_marker",
                "echo_message",
                "event_playback",
                "extended_isupport",
                "extended_join",
                "invite_notify",
                "invite_rules",
                "isupport",
                "kick",
                "kick_rules",
                "labeled_response",
                "message_tags",
                "metadata_error",
                "metadata_subscriptions",
                "monitor",
                "multi_prefix",
                "multiple_joins",
                "nick_change_and_collision",
                "part",
                "part_multiple_and_refusal",
                "persistence",
                "ping",
                "read_marker_on_join",
                "registration",
                "replies_and_reactions",
                "sasl_failure",
                "sasl_plain",
                "sasl_protocol_errors",
                "sasl_scram_sha_256",
                "server_information",
                "server_time",
                "setname",
                "standard_replies",
                "status_message",
                "sts",
                "tls",
                "tls_certificate_rejected",
                "topic",
                "userhost_in_names",
                "utf8only",
                "webirc",
                "websocket",
                "websocket_tls",
                "who_and_whois",
                "whox",
            ]
        );
    }
}
