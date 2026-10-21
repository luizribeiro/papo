//! Graceful process shutdown against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

use std::{io::Read as _, process::Stdio, time::Duration};

use papo::{Client, Event};
use papo_test_server::{from_env, unique};
use tokio::time::{sleep, timeout};

#[tokio::test(flavor = "current_thread")]
async fn sigterm_sends_quit_and_exits_successfully() {
    timeout(Duration::from_secs(20), async {
        let server = from_env();
        let nick = unique("papo-shutdown");
        let server_url = format!("irc://{}", server.plaintext_addr());
        let (observer, mut events) = Client::builder(server_url.clone())
            .nick(unique("papo-observer"))
            .connect()
            .await
            .unwrap();
        observer.monitor().add([&nick]).await.unwrap();

        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_graceful-shutdown"))
            .arg(server_url)
            .env("IRC_NICK", &nick)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        wait_for(
            &mut events,
            |event| matches!(event, Event::MonitorOnline(online) if online.nick == nick.as_str()),
        )
        .await;
        sleep(Duration::from_millis(100)).await;

        let signal = std::process::Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(signal.success());
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            sleep(Duration::from_millis(10)).await;
        };
        assert!(status.success());
        wait_for(&mut events, |event| {
            matches!(event, Event::MonitorOffline(offline) if offline.nick == nick.as_str())
        })
        .await;

        let mut stderr = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert_eq!(stderr, "shutting down\n");
        observer.quit("shutdown test complete").await.unwrap();
    })
    .await
    .expect("graceful shutdown test did not finish within 20 seconds");
}

async fn wait_for(events: &mut papo::Events, mut wanted: impl FnMut(&Event) -> bool) {
    while let Some(event) = events.next().await {
        if wanted(&event) {
            return;
        }
    }
    panic!("driver stopped before the expected event");
}
