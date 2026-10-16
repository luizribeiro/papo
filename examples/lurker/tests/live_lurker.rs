//! Lurker output against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

mod support;

use std::time::Duration;

use papo::{Client, Event};
use papo_proto::Message;
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
        let address = format!("irc://{}", server.plaintext_addr());

        let (owner, mut owner_events) = Client::builder(address.as_str())
            .nick(&owner_nick)
            .channels([&channel])
            .connect()
            .await
            .unwrap();
        wait_event(&mut owner_events, |event| synced(event, &channel)).await;
        assert!(owner.state(|state| state
            .channel(&channel)
            .unwrap()
            .member(&owner_nick)
            .unwrap()
            .is_op()));

        register_account(&server, &lurker_nick, PASSWORD).unwrap();
        let mut lurker = Lurker::start(&server, &lurker_nick, &channel);
        wait_event(&mut owner_events, |event| joined(event, &lurker_nick)).await;
        lurker.wait_for(&format!("synced {channel}:")).await;

        let (user, mut user_events) = Client::builder(address.as_str())
            .nick(&user_nick)
            .channels([&channel])
            .connect()
            .await
            .unwrap();
        wait_event(&mut user_events, |event| synced(event, &channel)).await;
        wait_event(&mut owner_events, |event| joined(event, &user_nick)).await;

        owner
            .send(Message::new("MODE", [&channel, "+o", &user_nick]))
            .await
            .unwrap();
        wait_event(&mut owner_events, |event| mode(event, 'o')).await;
        user.send(Message::new("AWAY", ["testing away"]))
            .await
            .unwrap();
        wait_event(&mut owner_events, |event| matches!(event, Event::AwayChanged { nick, .. } if nick == &user_nick)).await;
        user.send(Message::new("NICK", [&renamed])).await.unwrap();
        wait_event(&mut owner_events, |event| matches!(event, Event::NickChanged { new, .. } if new == &renamed)).await;
        user.send(Message::new("MODE", [&channel, "+v", &renamed]))
            .await
            .unwrap();
        wait_event(&mut owner_events, |event| mode(event, 'v')).await;
        user.send(Message::new("TOPIC", [&channel, "lurker topic"]))
            .await
            .unwrap();
        wait_event(&mut owner_events, |event| matches!(event, Event::TopicChanged { topic, .. } if topic.as_deref() == Some("lurker topic"))).await;
        user.send(Message::new("PART", [&channel, "lurker test complete"]))
            .await
            .unwrap();
        wait_event(&mut owner_events, |event| matches!(event, Event::Parted { nick, .. } if nick == &renamed)).await;
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
        user.quit(Some("lurker test complete")).await.unwrap();
        owner.quit(Some("lurker test complete")).await.unwrap();
    })
    .await
    .expect("live lurker test did not finish within 20 seconds");
}

async fn wait_event(events: &mut papo::Events, mut wanted: impl FnMut(&Event) -> bool) {
    loop {
        match events.next().await {
            Some(event) if wanted(&event) => return,
            Some(Event::Disconnected { reason, .. }) => panic!("client disconnected: {reason}"),
            Some(_) => {}
            None => panic!("driver stopped before the expected event"),
        }
    }
}

fn synced(event: &Event, channel: &str) -> bool {
    matches!(event, Event::ChannelSynced { channel: synced } if synced == channel)
}

fn joined(event: &Event, nick: &str) -> bool {
    matches!(event, Event::Joined { nick: joined, .. } if joined == nick)
}

fn mode(event: &Event, mode: char) -> bool {
    matches!(event, Event::ModeChanged { changes, .. } if changes.iter().any(|change| change.set && change.mode == mode))
}
