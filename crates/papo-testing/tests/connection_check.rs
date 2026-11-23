//! An in-memory server silently stops answering after IRC registration.

mod support {
    pub mod direct;
}

use std::time::Duration;

use papo::{Client, DisconnectCause, Error, Event, Reconnect};
use papo_testing::wire;
use support::direct::connected;
use tokio::time::{Instant, advance};

#[tokio::test(start_paused = true)]
async fn a_silent_connection_closes_at_the_deadline_and_retries_immediately() {
    let (client, mut events, mut wire) = wire(
        Client::builder("irc://offline.test", "papo")
            .rate_limit(
                papo::RateLimit::new(1, Duration::from_secs(60))
                    .unwrap()
                    .strict(),
            )
            .reconnect(Reconnect::exponential().initial_delay(Duration::from_secs(60))),
    );
    wire.register(&[]).await;
    connected(&mut events).await;

    client
        .send_raw(papo::Line::new("PING").param("rate-fence"))
        .await
        .unwrap();
    assert_eq!(wire.next_sent().await, "PING rate-fence");
    let queued = tokio::spawn(client.send_raw(papo::Line::privmsg("#papo", "queued update")));
    tokio::task::yield_now().await;
    let check = tokio::spawn(client.check_connection(Duration::from_secs(3)));
    assert_eq!(wire.next_sent().await, "PING papo-check-0");
    let started = Instant::now();
    advance(Duration::from_millis(2999)).await;
    assert!(!check.is_finished());
    assert!(client.snapshot().is_connected());
    advance(Duration::from_millis(1)).await;

    let Err(Error::Interrupted(interrupted)) = check.await.unwrap() else {
        panic!("expected an interrupted connection check");
    };
    assert_eq!(interrupted.cause, DisconnectCause::PingTimeout);
    let Err(Error::Disconnected(unsent)) = queued.await.unwrap() else {
        panic!("expected the paced line to remain unsent");
    };
    assert_eq!(unsent.cause, DisconnectCause::PingTimeout);
    let Some((Event::Disconnected(disconnected), _)) = events.next().await else {
        panic!("expected the silent connection to disconnect");
    };
    assert_eq!(disconnected.cause, DisconnectCause::PingTimeout);
    assert_eq!(disconnected.reconnect_in, Some(Duration::ZERO));
    assert_eq!(wire.next_sent_or_closed().await, None);

    wire.close().await;
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(
        Instant::now().duration_since(started),
        Duration::from_secs(3)
    );
}
