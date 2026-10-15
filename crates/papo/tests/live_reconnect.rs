//! Reconnection behavior against a process-owned Ergo server.
#![cfg(not(target_family = "wasm"))]

use std::time::Duration;

use papo::{Client, Event, Reconnect};
use papo_test_server::{TestServer, unique};
use tokio::time::timeout;

#[tokio::test(flavor = "current_thread")]
async fn reconnects_and_rejoins_after_ergo_restarts() {
    // This needs exclusive server lifecycle control, so it owns a TestServer.
    timeout(Duration::from_secs(20), async {
        let mut server = TestServer::start().unwrap();
        let nick = unique("papo-reconnect");
        let channel = format!("#{}", unique("papo-reconnect"));
        let (client, mut events) = Client::builder(&format!(
            "irc://localhost:{}",
            server.plaintext_addr().port()
        ))
        .nick(&nick)
        .channels([&channel])
        .reconnect(Reconnect::exponential().max_delay(Duration::from_secs(1)))
        .connect()
        .await
        .unwrap();

        wait_for(&mut events, |event| matches!(event, Event::Registered(_))).await;
        wait_for(&mut events, |event| joined(event, &nick, &channel)).await;
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
        server.restart().unwrap();
        wait_for(&mut events, |event| matches!(event, Event::Reconnected(_))).await;
        wait_for(&mut events, |event| joined(event, &nick, &channel)).await;
        client.quit(Some("reconnect test complete")).await.unwrap();
    })
    .await
    .expect("live reconnect test did not finish within 20 seconds");
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
    let Event::Message(message) = event else {
        return false;
    };
    let message = message.as_ref();
    message.command_is("JOIN")
        && message.param(0) == Some(channel)
        && message
            .raw_source()
            .is_some_and(|source| source == nick || source.starts_with(&format!("{nick}!")))
}
