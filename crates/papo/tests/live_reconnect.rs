//! Reconnection behavior against a process-owned Ergo server.
#![cfg(not(target_family = "wasm"))]

use std::time::Duration;

use papo::{Client, Event, Origin, Reconnect, Sasl};
use papo_test_server::{TestServer, register_account_at, unique};
use tokio::time::timeout;

#[tokio::test(flavor = "current_thread")]
async fn reconnects_and_rejoins_after_ergo_restarts() {
    // This needs exclusive server lifecycle control, so it owns a TestServer.
    timeout(Duration::from_secs(20), async {
        let mut server = TestServer::start().unwrap();
        let nick = unique("papo-reconnect");
        let channel = format!("#{}", unique("papo-reconnect"));
        let password = "q7Hx-2mZp_9vLcT4wRfB";
        register_account_at(server.plaintext_addr(), &nick, password).unwrap();
        let (client, mut events) = Client::builder(&format!(
            "irc://localhost:{}",
            server.plaintext_addr().port()
        ))
        .nick(&nick)
        .sasl(Sasl::plain(&nick, password))
        .require_sasl(true)
        .caps([
            "message-tags",
            "server-time",
            "batch",
            "echo-message",
            "labeled-response",
            "draft/chathistory",
        ])
        .channels([&channel])
        .history_on_reconnect(true)
        .reconnect(Reconnect::exponential().max_delay(Duration::from_secs(5)))
        .connect()
        .await
        .unwrap();

        wait_for(&mut events, |event| matches!(event, Event::Registered(_))).await;
        wait_for(&mut events, |event| joined(event, &nick, &channel)).await;
        client
            .privmsg(&channel, "seen before the outage")
            .await
            .unwrap();
        server.stop().unwrap();
        wait_for(&mut events, |event| {
            matches!(
                event,
                Event::Disconnected {
                    will_retry: true,
                    ..
                }
            )
        })
        .await;
        wait_for(&mut events, |event| {
            matches!(
                event,
                Event::Disconnected {
                    will_retry: true,
                    ..
                }
            )
        })
        .await;
        wait_for(&mut events, |event| {
            matches!(
                event,
                Event::Disconnected {
                    will_retry: true,
                    ..
                }
            )
        })
        .await;
        server.restart().unwrap();
        let poster = post_during_outage(&server, &channel).await;

        wait_for(&mut events, |event| matches!(event, Event::Reconnected(_))).await;
        wait_for(&mut events, |event| joined(event, &nick, &channel)).await;
        let messages = wait_history(&mut events, &channel).await;
        let missed = messages
            .iter()
            .filter(|message| message.text().contains("missed during the outage"))
            .collect::<Vec<_>>();
        assert_eq!(missed.len(), 2);
        assert!(
            missed
                .iter()
                .all(|message| message.origin() == Origin::History)
        );
        poster.quit(Some("messages delivered")).await.unwrap();
        client.quit(Some("reconnect test complete")).await.unwrap();
    })
    .await
    .expect("live reconnect test did not finish within 20 seconds");
}

async fn post_during_outage(server: &TestServer, channel: &str) -> Client {
    let poster_nick = unique("papo-reconnect-poster");
    let (poster, mut events) = Client::builder(&format!(
        "irc://localhost:{}",
        server.plaintext_addr().port()
    ))
    .nick(&poster_nick)
    .channels([channel])
    .connect()
    .await
    .unwrap();
    wait_for(
        &mut events,
        |event| matches!(event, Event::ChannelSynced { channel: synced } if synced == channel),
    )
    .await;
    poster
        .privmsg(channel, "missed during the outage")
        .await
        .unwrap();
    poster
        .privmsg(channel, "also missed during the outage")
        .await
        .unwrap();
    poster
}

async fn wait_history(events: &mut papo::Events, target: &str) -> Vec<papo::MessageEvent> {
    loop {
        match events.next().await {
            Some(Event::History {
                target: received,
                messages,
                ..
            }) if received == target => return messages,
            Some(Event::Disconnected { reason, .. }) => panic!("disconnected: {reason}"),
            Some(_) => {}
            None => panic!("driver stopped before history arrived"),
        }
    }
}

async fn wait_for(events: &mut papo::Events, accepted: impl Fn(&Event) -> bool) {
    loop {
        let event = events.next().await.expect("driver stopped unexpectedly");
        if accepted(&event) {
            return;
        }
    }
}

fn joined(event: &Event, nick: &str, channel: &str) -> bool {
    matches!(
        event,
        Event::Joined {
            channel: joined,
            nick: joined_nick,
        } if joined == channel && joined_nick == nick
    )
}
