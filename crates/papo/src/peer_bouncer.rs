use papo_core::NetworkState;

#[derive(Debug)]
pub(crate) struct PeerState {
    login: Option<String>,
}

impl PeerState {
    pub(crate) fn from_session(session: &papo_core::Session) -> Self {
        Self {
            login: session.__sasl_identity().map(str::to_owned),
        }
    }

    pub(crate) fn scope(
        &self,
        state: &papo_core::State,
        server: &str,
        network_id: &str,
    ) -> Option<crate::peer_key::PeerScope> {
        let login = self.login.as_deref()?;
        let network = state
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
}

fn format_scope(
    server: &str,
    login: &str,
    network_id: &str,
    upstream: &str,
    tls: Option<bool>,
    state: Option<NetworkState>,
) -> Option<crate::peer_key::PeerScope> {
    if tls != Some(true) || state != Some(NetworkState::Connected) {
        return None;
    }
    let upstream = crate::sts::normalize_host(upstream);
    Some(crate::peer_key::PeerScope {
        server: server.to_owned(),
        bouncer: Some(crate::peer_key::BouncerScope {
            login: login.to_owned(),
            network: network_id.to_owned(),
            upstream,
        }),
    })
}
