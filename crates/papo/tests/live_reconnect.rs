//! Reconnection behavior against a process-owned Ergo server.
#![cfg(not(target_family = "wasm"))]

mod support;

use support::within;

use std::time::Duration;

use papo::{Client, Endpoint, Event, Origin, Reconnect, Sasl, cap};
use papo_test_server::{TestServer, register_account_at, unique};

#[tokio::test(flavor = "current_thread")]
async fn fills_a_dropped_connection_gap_once_across_two_reconnects() {
    within(async {
        let server = TestServer::start_default().unwrap();
        let proxy = server.fault_proxy().unwrap();
        let nick = unique("papo-reconnect");
        let channel = format!("#{}", unique("papo-reconnect"));
        let password = "q7Hx-2mZp_9vLcT4wRfB";
        register_account_at(server.plaintext_addr(), &nick, password).unwrap();
        let builder =
            Client::builder(Endpoint::plaintext("localhost", proxy.address().port()).unwrap())
                .nick(support::nick(&nick))
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
                .reconnect(Reconnect::exponential().max_delay(Duration::from_secs(5)));
        #[cfg(feature = "unstable")]
        let builder = builder.without_caps([cap::PERSISTENCE]);
        let (client, mut events) = builder.connect().await.unwrap();

        wait_for(&mut events, "initial registration", |event| {
            matches!(event, Event::Connected(_))
        })
        .await;
        wait_for(&mut events, "initial join", |event| {
            joined(event, &nick, &channel)
        })
        .await;
        let poster = connect_poster(&server, &channel).await;
        client
            .privmsg(&channel, "seen before the outage")
            .await
            .unwrap();
        proxy.drop_connection().unwrap();
        wait_for(&mut events, "dropped connection", |event| {
            matches!(
                event,
                Event::Disconnected(disconnected) if disconnected.is_retrying()
            )
        })
        .await;
        poster
            .privmsg(&channel, "missed during the outage")
            .await
            .unwrap();

        wait_for(
            &mut events,
            "first reconnection",
            |event| matches!(event, Event::Connected(connected) if connected.reconnected),
        )
        .await;
        wait_for(&mut events, "first rejoin", |event| {
            joined(event, &nick, &channel)
        })
        .await;
        let history = wait_history(&mut events, &channel).await;
        let missed = history
            .messages()
            .filter(|message| message.text.contains("missed during the outage"))
            .collect::<Vec<_>>();
        assert_eq!(missed.len(), 1);
        assert!(
            missed
                .iter()
                .all(|message| message.origin == Origin::History)
        );
        let delivered_ids = history
            .messages()
            .filter_map(|message| message.msgid.as_ref().map(ToString::to_string))
            .collect::<Vec<_>>();
        reconnect_and_rejoin(&client, &mut events, &nick, &channel).await;
        assert_no_repeated_history(&mut events, &channel, &delivered_ids).await;
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
    wait_for(events, "explicit disconnect", |event| {
        matches!(event, Event::Disconnected(_))
    })
    .await;
    wait_for(
        events,
        "explicit reconnection",
        |event| matches!(event, Event::Connected(connected) if connected.reconnected),
    )
    .await;
    wait_for(events, "second rejoin", |event| {
        joined(event, nick, channel)
    })
    .await;
}

async fn connect_poster(server: &TestServer, channel: &str) -> Client {
    let poster_nick = unique("papo-reconnect-poster");
    let (poster, mut events) =
        Client::builder(Endpoint::plaintext("localhost", server.plaintext_addr().port()).unwrap())
            .nick(support::nick(&poster_nick))
            .autojoin([channel])
            .connect()
            .await
            .unwrap();
    wait_for(
        &mut events,
        "poster channel sync",
        |event| matches!(event, Event::ChannelSynced(papo::event::ChannelSynced { channel: synced, .. }) if synced == channel),
    )
    .await;
    poster
}

async fn wait_history(events: &mut papo::Events, target: &str) -> papo::HistoryPage {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.next().await {
                Some(Event::History(history))
                    if matches!(&history.conversation, papo::Conversation::Channel(channel) if channel == target) =>
                {
                    return *history;
                }
                Some(Event::Disconnected(disconnected)) => {
                    panic!("disconnected: {disconnected}")
                }
                Some(_) => {}
                None => panic!("driver stopped before history arrived"),
            }
        }
    })
    .await
    .expect("timed out waiting for reconnect history")
}

async fn assert_no_repeated_history(
    events: &mut papo::Events,
    target: &str,
    delivered_ids: &[String],
) {
    let deadline = tokio::time::sleep(Duration::from_secs(1));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            () = &mut deadline => return,
            event = events.next() => match event {
                Some(Event::History(history))
                    if matches!(&history.conversation, papo::Conversation::Channel(channel) if channel == target) => assert!(
                    history.messages().all(|message| {
                        !message.text.contains("missed during the outage")
                            && message.msgid.as_ref().is_none_or(|msgid| {
                                !delivered_ids.iter().any(|seen| seen.as_str() == msgid.as_ref())
                            })
                    }),
                    "repeated catch-up item: {:#?}",
                    history.items,
                ),
                Some(Event::Disconnected(disconnected)) => {
                    panic!("disconnected: {disconnected}")
                }
                Some(_) => {}
                None => panic!("driver stopped while checking repeated history"),
            },
        }
    }
}

async fn wait_for(events: &mut papo::Events, operation: &str, accepted: impl Fn(&Event) -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.next().await.expect("driver stopped unexpectedly");
            if accepted(&event) {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {operation}"));
}

fn joined(event: &Event, nick: &str, channel: &str) -> bool {
    matches!(
        event,
        Event::Joined(papo::event::Joined { channel: joined,
            nick: joined_nick, .. }) if joined == channel && joined_nick == nick
    )
}
