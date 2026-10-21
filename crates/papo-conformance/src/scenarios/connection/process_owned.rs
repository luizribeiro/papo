use papo::{Client, Endpoint};
use papo_core::NetworkConfig;

use crate::Scenario;

pub(super) async fn bouncer_networks(s: Scenario) -> papo_testing::Result<()> {
    let Some(bouncer) = s.server.bouncer() else {
        return s.skip("the selected server is not a managed bouncer");
    };
    let network_nick = papo_test_server::unique("alice");
    let login = bouncer.provision_network(&network_nick)?;
    let address = s.server.plaintext_addr();
    let alice = s
        .connect_with(
            Client::builder(Endpoint::plaintext(address.ip().to_string(), address.port()).unwrap())
                .nick(network_nick)
                .username(login.username())
                .realname("papo bouncer control")
                .password(login.password())
                .enable(papo::Feature::Bouncer)
                .request_caps(["batch", "soju.im/bouncer-networks"]),
        )
        .await?;
    let initial = alice.bouncer().networks().await?;
    assert_eq!(
        alice
            .state()
            .bouncer_networks()
            .cloned()
            .collect::<Vec<_>>(),
        initial
    );

    let upstream = bouncer.upstream_addr();
    let host = upstream.ip().to_string();
    let name = papo_test_server::unique("managed-network");
    let config = NetworkConfig::new(host.clone())
        .port(upstream.port())
        .tls(false)
        .name(name.clone())
        .nickname(
            alice
                .state()
                .casemapping()
                .nick(papo_test_server::unique("upstream-alice")),
        );
    let id = alice.bouncer().add(config).await?;
    let added = alice.bouncer().networks().await?;
    let network = added
        .iter()
        .find(|network| network.id == id)
        .expect("the added network is listed");
    assert_eq!(network.id, id);
    assert_eq!(network.name.as_deref(), Some(name.as_str()));
    assert_eq!(network.host.as_deref(), Some(host.as_str()));
    assert_eq!(network.port, Some(upstream.port()));

    let changed_name = papo_test_server::unique("renamed-network");
    let changed = NetworkConfig::new(host)
        .port(upstream.port())
        .tls(false)
        .name(changed_name.clone())
        .nickname(
            alice
                .state()
                .casemapping()
                .nick(papo_test_server::unique("upstream-bob")),
        );
    alice.bouncer().change(&id, changed).await?;
    let changed = alice.bouncer().networks().await?;
    assert_eq!(
        changed
            .iter()
            .find(|network| network.id == id)
            .and_then(|network| network.name.as_deref()),
        Some(changed_name.as_str())
    );

    alice.bouncer().remove(&id).await?;
    let removed = alice.bouncer().networks().await?;
    assert!(
        removed.iter().all(|network| network.id != id),
        "the removed network was still listed"
    );
    Ok(())
}
