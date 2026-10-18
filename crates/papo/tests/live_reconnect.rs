//! Reconnection behavior against a process-owned Ergo server.
#![cfg(not(target_family = "wasm"))]

mod support;

use support::within;

use std::time::Duration;

use papo::{Client, Endpoint, Event, Origin, Reconnect, Sasl, cap};
use papo_test_server::{TestServer, register_account_at, unique};

#[tokio::test(flavor = "current_thread")]
async fn reconnects_and_rejoins_after_ergo_restarts() {
    // This needs exclusive server lifecycle control, so it owns a TestServer.
    within(async {
        let mut server = TestServer::start().unwrap();
        let nick = unique("papo-reconnect");
        let channel = format!("#{}", unique("papo-reconnect"));
        let password = "q7Hx-2mZp_9vLcT4wRfB";
        register_account_at(server.plaintext_addr(), &nick, password).unwrap();
        let (client, mut events) = Client::builder(Endpoint::plaintext(
            "localhost",
            server.plaintext_addr().port(),
        ))
        .nick(&nick)
        .sasl(Sasl::plain(&nick, password))
        .request_caps([
            "message-tags",
            "server-time",
            "batch",
            "echo-message",
            "labeled-response",
            "draft/chathistory",
        ])
        .without_caps([cap::EVENT_PLAYBACK, cap::MULTILINE])
        .autojoin([&channel])
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
                Event::Disconnected(disconnected) if disconnected.is_retrying()
            )
        })
        .await;
        wait_for(&mut events, |event| {
            matches!(
                event,
                Event::Disconnected(disconnected) if disconnected.is_retrying()
            )
        })
        .await;
        wait_for(&mut events, |event| {
            matches!(
                event,
                Event::Disconnected(disconnected) if disconnected.is_retrying()
            )
        })
        .await;
        server.restart().unwrap();
        let poster = post_during_outage(&server, &channel).await;

        wait_for(
            &mut events,
            |event| matches!(event, Event::Registered(registered) if registered.reconnected),
        )
        .await;
        wait_for(&mut events, |event| joined(event, &nick, &channel)).await;
        let history = wait_history(&mut events, &channel).await;
        let missed = history
            .messages()
            .filter(|message| message.text().contains("missed during the outage"))
            .collect::<Vec<_>>();
        assert_eq!(missed.len(), 1);
        assert!(
            missed
                .iter()
                .all(|message| message.origin() == Origin::History)
        );
        let delivered_ids = history
            .messages()
            .filter_map(|message| message.msgid().map(str::to_owned))
            .collect::<Vec<_>>();
        reconnect_and_rejoin(&client, &mut events, &nick, &channel).await;
        let repeated = wait_history(&mut events, &channel).await;
        assert!(
            repeated.messages().all(|message| {
                !message.text().contains("missed during the outage")
                    && message
                        .msgid()
                        .is_none_or(|msgid| !delivered_ids.iter().any(|seen| seen == msgid))
            }),
            "repeated catch-up item: {:#?}",
            repeated.items,
        );
        poster.quit("messages delivered").await.unwrap();
        client.quit("reconnect test complete").await.unwrap();
    })
    .await;
}

async fn reconnect_and_rejoin(
    client: &Client,
    events: &mut papo::Events,
    nick: &str,
    channel: &str,
) {
    client.reconnect().await.unwrap();
    wait_for(events, |event| matches!(event, Event::Disconnected(_))).await;
    wait_for(
        events,
        |event| matches!(event, Event::Registered(registered) if registered.reconnected),
    )
    .await;
    wait_for(events, |event| joined(event, nick, channel)).await;
}

async fn post_during_outage(server: &TestServer, channel: &str) -> Client {
    let poster_nick = unique("papo-reconnect-poster");
    let (poster, mut events) = Client::builder(Endpoint::plaintext(
        "localhost",
        server.plaintext_addr().port(),
    ))
    .nick(&poster_nick)
    .autojoin([channel])
    .connect()
    .await
    .unwrap();
    wait_for(
        &mut events,
        |event| matches!(event, Event::ChannelSynced(papo::event::ChannelSynced { channel: synced, .. }) if synced == channel),
    )
    .await;
    poster
        .privmsg(channel, "missed during the outage")
        .await
        .unwrap();
    poster
}

async fn wait_history(events: &mut papo::Events, target: &str) -> papo::HistoryPage {
    loop {
        match events.next().await {
            Some(Event::History(history)) if history.target == target => {
                return *history;
            }
            Some(Event::Disconnected(disconnected)) => {
                panic!("disconnected: {disconnected}")
            }
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
        Event::Joined(papo::event::Joined { channel: joined,
            nick: joined_nick, .. }) if joined == channel && joined_nick == nick
    )
}
