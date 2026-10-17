//! Lurker output against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

mod support;

use std::time::Duration;

use papo::{Client, Endpoint, Event};
use papo_test_server::{from_env, register_account, unique};
use support::{Lurker, PASSWORD};
use tokio::time::timeout;

#[tokio::test(flavor = "current_thread")]
async fn prints_channel_state_changes_in_wire_order() {
    timeout(Duration::from_secs(20), async {
        let server = from_env();
        let channel = format!("#{}", unique("papo-lurk"));
        let owner_nick = unique("papo-owner");
        let lurker_nick = unique("papo-lurker");
        let user_nick = unique("papo-user");
        let renamed = unique("papo-renamed");
        let endpoint = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        );

        let (owner, mut owner_events) = Client::builder(endpoint.clone())
            .nick(&owner_nick)
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_event(&mut owner_events, |event| synced(event, &channel)).await;
        assert!(owner
            .state()
            .channel(&channel)
            .unwrap()
            .member(&owner_nick)
            .unwrap()
            .is_op());

        register_account(&server, &lurker_nick, PASSWORD).unwrap();
        let mut lurker = Lurker::start(&server, &lurker_nick, &channel);
        wait_event(&mut owner_events, |event| joined(event, &lurker_nick)).await;
        lurker.wait_for(&format!("synced {channel}:")).await;

        let (user, mut user_events) = Client::builder(endpoint)
            .nick(&user_nick)
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_event(&mut user_events, |event| synced(event, &channel)).await;
        wait_event(&mut owner_events, |event| joined(event, &user_nick)).await;

        owner
            .send_raw(&format!("MODE {channel} +o {user_nick}"))
            .await
            .unwrap();
        wait_event(&mut owner_events, |event| mode(event, 'o')).await;
        user.send_raw("AWAY :testing away")
            .await
            .unwrap();
        wait_event(&mut owner_events, |event| matches!(event, Event::AwayChanged(changed) if changed.nick == user_nick.as_str())).await;
        user.send_raw(&format!("NICK {renamed}")).await.unwrap();
        wait_event(&mut owner_events, |event| matches!(event, Event::NickChanged(changed) if changed.new == renamed.as_str())).await;
        user.send_raw(&format!("MODE {channel} +v {renamed}"))
            .await
            .unwrap();
        wait_event(&mut owner_events, |event| mode(event, 'v')).await;
        user.send_raw(&format!("TOPIC {channel} :lurker topic"))
            .await
            .unwrap();
        wait_event(&mut owner_events, |event| matches!(event, Event::TopicChanged(changed) if changed.topic.as_deref() == Some("lurker topic"))).await;
        user.send_raw(&format!("PART {channel} :lurker test complete"))
            .await
            .unwrap();
        wait_event(&mut owner_events, |event| matches!(event, Event::Parted(parted) if parted.nick == renamed.as_str())).await;
        lurker.wait_for(&format!("part {channel} {renamed}:")).await;

        let output = lurker.finish();
        assert_eq!(
            output.lines().collect::<Vec<_>>(),
            [
                format!("synced {channel}: @{owner_nick}[account=?] {lurker_nick}[account={lurker_nick}]"),
                format!("join {channel} {user_nick}"),
                format!("mode {channel} by {owner_nick}: +o={user_nick}"),
                format!("away {user_nick}: testing away"),
                format!("nick {user_nick} -> {renamed}"),
                format!("mode {channel} by {renamed}: +v={renamed}"),
                format!("topic {channel} by {renamed}: lurker topic"),
                format!("part {channel} {renamed}: lurker test complete"),
            ]
        );
        user.quit("lurker test complete").await.unwrap();
        owner.quit("lurker test complete").await.unwrap();
    })
    .await
    .expect("live lurker test did not finish within 20 seconds");
}

async fn wait_event(events: &mut papo::Events, mut wanted: impl FnMut(&Event) -> bool) {
    loop {
        match events.next().await {
            Some(event) if wanted(&event) => return,
            Some(Event::Disconnected(disconnected)) => {
                panic!("client disconnected: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped before the expected event"),
        }
    }
}

fn synced(event: &Event, channel: &str) -> bool {
    matches!(event, Event::ChannelSynced(papo::event::ChannelSynced { channel: synced, .. }) if synced == channel)
}

fn joined(event: &Event, nick: &str) -> bool {
    matches!(event, Event::Joined(papo::event::Joined { nick: joined, .. }) if joined == nick)
}

fn mode(event: &Event, mode: char) -> bool {
    matches!(event, Event::ModeChanged(changed) if changed.changes.iter().any(|change| change.set && change.mode == mode))
}
