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
