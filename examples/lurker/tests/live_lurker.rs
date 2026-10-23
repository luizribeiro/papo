//! Lurker output against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

mod support;

use std::time::Duration;

use papo::{Client, Event};
use papo_test_server::{from_env, unique};
use support::Lurker;
use tokio::time::timeout;

const CHANNEL: &str = "#papo";

#[tokio::test(flavor = "current_thread")]
async fn prints_membership_snapshot_and_changes() {
    timeout(Duration::from_secs(20), async {
        let server = from_env();
        let observer_nick = unique("papo-observer");
        let user_nick = unique("papo-user");
        let peer_nick = unique("papo-peer");
        let renamed = unique("papo-renamed");
        let server_url = format!("irc://{}", server.plaintext_addr());
        let (user, mut events) = Client::builder(server_url.clone())
            .nick(&user_nick)
            .autojoin([CHANNEL])
            .connect()
            .await
            .unwrap();
        wait_for(&mut events, |event| {
            matches!(event, Event::ChannelSynced(_))
        })
        .await;

        let mut lurker = Lurker::start(&server_url, &observer_nick);
        wait_for(
            &mut events,
            |event| matches!(event, Event::Joined(joined) if joined.nick == observer_nick.as_str()),
        )
        .await;
        lurker.wait_for("members #papo:").await;

        let (peer, mut peer_events) = Client::builder(server_url)
            .nick(&peer_nick)
            .autojoin([CHANNEL])
            .connect()
            .await
            .unwrap();
        wait_for(&mut peer_events, |event| {
            matches!(event, Event::ChannelSynced(_))
        })
        .await;
        wait_for(&mut events, |event| matches!(event, Event::Joined(_))).await;
        peer.set_nick(&renamed).await.unwrap();
        peer.part(CHANNEL, "leaving the example").await.unwrap();
        lurker.wait_for(&format!("part {CHANNEL} {renamed}")).await;
        user.quit("lurker test complete").await.unwrap();
        lurker.wait_for(&format!("quit {user_nick}")).await;

        let output = lurker.finish();
        assert!(output.contains(&format!("members {CHANNEL}:")));
        assert!(output.contains(&format!("join {CHANNEL} {peer_nick}")));
        assert!(output.contains(&format!("nick {peer_nick} -> {renamed}")));
        assert!(output.contains(&format!("part {CHANNEL} {renamed}")));
        assert!(output.contains(&format!("quit {user_nick}")));
    })
    .await
    .expect("live lurker test did not finish within 20 seconds");
}

async fn wait_for(events: &mut papo::Events, mut wanted: impl FnMut(&Event) -> bool) {
    while let Some(event) = events.next().await {
        if wanted(&event) {
            return;
        }
    }
    panic!("driver stopped before the expected event");
}
