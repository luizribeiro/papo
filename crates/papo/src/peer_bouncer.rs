use papo_core::NetworkState;

pub(crate) fn scope(state: &crate::State, server: &str, network_id: &str) -> Option<String> {
    let login = state.bouncer_login.as_deref()?;
    let network = state
        .core
        .bouncer_networks()
        .find(|network| network.id == network_id)?;
    format_scope(
        server,
        login,
        network_id,
        network.host.as_deref()?,
        network.tls,
        network.state,
    )
}

fn format_scope(
    server: &str,
    login: &str,
    network_id: &str,
    upstream: &str,
    tls: Option<bool>,
    state: Option<NetworkState>,
) -> Option<String> {
    if tls != Some(true) || state != Some(NetworkState::Connected) {
        return None;
    }
    let upstream = crate::sts::normalize_host(upstream);
    Some(format!(
        "server={}:{server};login={}:{login};network={}:{network_id};upstream={}:{upstream}",
        server.len(),
        login.len(),
        network_id.len(),
        upstream.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::format_scope;
    use papo_core::NetworkState;

    #[test]
    fn durable_scope_requires_every_bound_tls_upstream_fact() {
        let scope = |login, network, host, tls, state| {
            format_scope("bouncer.example", login, network, host, tls, state)
        };
        let connected = Some(NetworkState::Connected);
        let expected = scope(
            "alice",
            "libera-42",
            "IRC.Libera.Chat.",
            Some(true),
            connected,
        )
        .unwrap();
        assert_eq!(
            expected,
            "server=15:bouncer.example;login=5:alice;network=9:libera-42;upstream=15:irc.libera.chat"
        );
        assert_ne!(
            scope("alice", "oftc-17", "irc.oftc.net", Some(true), connected),
            Some(expected.clone())
        );
        assert_eq!(
            scope("alice", "libera-42", "irc.libera.chat", None, connected),
            None
        );
        assert_eq!(
            scope(
                "alice",
                "libera-42",
                "irc.libera.chat",
                Some(false),
                connected
            ),
            None
        );
        assert_eq!(
            scope(
                "alice",
                "libera-42",
                "irc.libera.chat",
                Some(true),
                Some(NetworkState::Disconnected)
            ),
            None
        );
    }
}
