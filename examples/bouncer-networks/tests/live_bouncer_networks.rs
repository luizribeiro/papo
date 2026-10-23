//! Bouncer-network example output against the runner-managed soju and Ergo servers.
#![cfg(not(target_family = "wasm"))]

mod support;

use std::time::Duration;

use papo::{Client, Event, Feature, Sasl};
use papo_test_server::{from_env, unique};
use support::BouncerExample;
use tokio::time::timeout;

#[tokio::test(flavor = "current_thread")]
async fn lists_adds_and_attaches_to_bouncer_networks() {
    timeout(Duration::from_secs(30), async {
        let server = from_env();
        let Some(bouncer) = server.bouncer() else {
            return;
        };
        let url = format!("irc://{}", server.plaintext_addr());
        let upstream = format!("irc://{}", bouncer.upstream_addr());
        let nick = unique("papo-bouncer-example");
        let mut example = BouncerExample::start(
            &url,
            &upstream,
            &nick,
            bouncer.username(),
            bouncer.password(),
        );
        example.wait_for("attached ").await;
        let stdout = example.finish().await;
        let (manager, _events) = Client::builder(url.clone())
            .nick(&nick)
            .sasl(Sasl::plain(bouncer.username(), bouncer.password()))
            .allow_insecure_auth(true)
            .enable(Feature::Bouncer)
            .connect()
            .await
            .unwrap();
        let networks = manager.bouncer().networks().await.unwrap();
        let added = networks
            .iter()
            .find(|network| network.name.as_deref() == Some("second"))
            .expect("the example's added network is listed");
        let id = added.id.clone();
        let existing = networks
            .iter()
            .find(|network| network.name.as_deref() == Some("ergo"))
            .expect("the bouncer's original network is listed");
        assert_eq!(
            stdout.lines().collect::<Vec<_>>(),
            [
                format!(
                    "network {}: ergo {} connected",
                    existing.id,
                    bouncer.upstream_addr().ip()
                ),
                format!(
                    "changed {id}: second {} disconnected",
                    bouncer.upstream_addr().ip()
                ),
                format!(
                    "changed {id}: second {} connected",
                    bouncer.upstream_addr().ip()
                ),
                format!("attached {id} #papo"),
            ]
        );

        let (attached, mut events) = Client::builder(url)
            .nick(&nick)
            .sasl(Sasl::plain(bouncer.username(), bouncer.password()))
            .allow_insecure_auth(true)
            .bouncer_network(id.clone())
            .autojoin(["#papo"])
            .connect()
            .await
            .unwrap();
        assert_eq!(attached.state().bouncer_network(), Some(&id));
        while !matches!(events.next().await, Some(Event::ChannelSynced(_))) {}
        attached.quit("bouncer example checked").await.unwrap();
        manager.quit("bouncer example checked").await.unwrap();
    })
    .await
    .expect("live bouncer-network example did not finish within 30 seconds");
}
