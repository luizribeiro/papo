//! Highest-layer queued direct recipient regression.

mod support {
    pub mod direct;
}

use std::time::Duration;

use papo::Reconnect;
use papo_testing::wire;
use support::direct::{connected, join, next_active_event};

#[tokio::test(start_paused = true)]
async fn queued_direct_line_is_rechecked_after_a_nickname_takeover() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::new(1, Duration::from_secs(60)).unwrap())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    join(&mut events, &mut wire, ":papo!p@localhost JOIN #review").await;
    let alice = join(
        &mut events,
        &mut wire,
        ":alice!~alice@cloak.example JOIN #review",
    )
    .await;
    join(
        &mut events,
        &mut wire,
        ":mallory!~mallory@users.example JOIN #review",
    )
    .await;
    client
        .send_raw(papo::Line::new("PING").param("fence"))
        .await
        .unwrap();
    assert_eq!(wire.next_sent().await, "PING fence");

    let queued = tokio::spawn({
        let client = client.clone();
        let alice = alice.clone();
        async move { client.privmsg(alice, "classified").await }
    });
    tokio::task::yield_now().await;
    wire.server(":alice!~alice@cloak.example QUIT :Client Quit")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::Quit(_)
    ));
    wire.server(":mallory!~mallory@users.example NICK alice")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::NickChanged(_)
    ));
    tokio::time::advance(Duration::from_secs(60)).await;

    assert!(matches!(
        queued.await.unwrap(),
        Err(papo::Error::TargetGone)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}
