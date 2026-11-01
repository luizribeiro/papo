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

mod tests {
    #[test]
    fn durable_scope_requires_every_bound_tls_upstream_fact() {
        use super::{NetworkState, format_scope};

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
        assert_eq!(expected.server, "bouncer.example");
        let bouncer = expected.bouncer.as_ref().unwrap();
        assert_eq!(bouncer.login, "alice");
        assert_eq!(bouncer.network, "libera-42");
        assert_eq!(bouncer.upstream, "irc.libera.chat");
        assert_ne!(
            scope("alice", "oftc-17", "irc.oftc.net", Some(true), connected),
            Some(expected)
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

    #[test]
    fn durable_scope_uses_the_selected_network() {
        use papo_core::{Config, Session};

        let now = crate::rt::now_mono();
        let mut session = Session::new(
            Config::default()
                .enable(papo_core::Feature::Bouncer)
                .wanted_caps(["soju.im/bouncer-networks"]),
        );
        session.handle_connected(now);
        session.handle_line(now, b"CAP * LS :soju.im/bouncer-networks");
        session.handle_line(now, b"CAP * ACK :soju.im/bouncer-networks");
        session.handle_line(
            now,
            b"BOUNCER NETWORK libera-42 host=IRC.Libera.Chat.;tls=1;state=connected",
        );
        session.handle_line(
            now,
            b"BOUNCER NETWORK oftc-17 host=irc.oftc.net;tls=1;state=connected",
        );
        let peer = super::PeerState {
            login: Some("alice".into()),
        };

        let scope = peer
            .scope(session.state(), "bouncer.example", "libera-42")
            .unwrap();
        assert_eq!(scope.server, "bouncer.example");
        let bouncer = scope.bouncer.unwrap();
        assert_eq!(bouncer.login, "alice");
        assert_eq!(bouncer.network, "libera-42");
        assert_eq!(bouncer.upstream, "irc.libera.chat");
        assert_eq!(
            peer.scope(session.state(), "bouncer.example", "missing-91"),
            None
        );
    }
}
