//! Offline coverage for command response boundaries.

mod support {
    pub mod direct;
}

use papo::{Error, Feature, ServerTime, UnsupportedReason};
use support::direct::connected;

#[tokio::test(start_paused = true)]
async fn neighboring_requests_without_labels() {
    let (client, mut events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(papo::Reconnect::never()),
    );
    wire.register(&[
        "draft/read-marker",
        "draft/metadata-2",
        "message-tags",
        "server-time",
    ])
    .await;
    connected(&mut events).await;

    let list = tokio::spawn({
        let client = client.clone();
        async move { client.list().await }
    });
    assert_eq!(wire.next_sent().await, "LIST");
    for line in [
        ":offline.test 321 papo Channel :Users Name",
        ":offline.test 322 papo #papo 3 :Release planning",
        ":offline.test 323 papo :End of /LIST",
        "@time=2026-10-15T12:00:01.000Z :alice!~a@cloak PRIVMSG papo :Deployment started",
    ] {
        wire.server(line).await;
    }
    assert_eq!(list.await.unwrap().unwrap().len(), 1);

    let time = ServerTime::from_stored_millis(1_792_065_600_000).unwrap();
    let mark = client.mark_read("#papo").up_to_time(time).await;
    let metadata = client.metadata();
    for error in [
        mark.unwrap_err(),
        metadata.list("#papo").await.unwrap_err(),
        metadata.list("papo").await.unwrap_err(),
    ] {
        assert!(matches!(error, Error::Unsupported(missing)
            if missing.feature == Feature::LabeledResponse
                && missing.reason == UnsupportedReason::NotOffered));
    }

    wire.server("PING :request-probe").await;
    assert_eq!(wire.next_sent().await, "PONG request-probe");
}
