//! Offline wire coverage for history correlation.

mod support {
    pub mod direct;
}

use papo::{Error, Event, Feature, ServerTime, UnsupportedReason};
use support::direct::{connected, next_active_event};

async fn connect(caps: &[&str]) -> (papo::Client, papo::Events, papo_testing::Wire) {
    let (client, mut events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(papo::Reconnect::never()),
    );
    wire.register(caps).await;
    connected(&mut events).await;
    (client, events, wire)
}

async fn request(client: &papo::Client, command: &str) -> Result<usize, Error> {
    let history = client.history("#papo");
    let first = ServerTime::from_stored_millis(1_760_529_301_000).unwrap();
    let last = ServerTime::from_stored_millis(1_760_529_401_000).unwrap();
    let page = match command {
        "LATEST" => history.latest(),
        "BEFORE" => history.before(last),
        "AFTER" => history.after(first),
        "AROUND" => history.around(first),
        "BETWEEN" => history.between(first, last),
        "TARGETS" => {
            return client
                .recent_conversations(first, last)
                .await
                .map(|rows| rows.len());
        }
        _ => unreachable!(),
    };
    page.await.map(|page| page.items.len())
}

#[tokio::test(start_paused = true)]
async fn history_request_capability_matrix() {
    for command in ["LATEST", "BEFORE", "AFTER", "AROUND", "BETWEEN", "TARGETS"] {
        for (labeled, batch) in [(false, false), (false, true), (true, false), (true, true)] {
            let mut caps = vec!["draft/chathistory", "server-time", "message-tags"];
            caps.extend(labeled.then_some("labeled-response"));
            caps.extend(batch.then_some("batch"));
            let (client, mut events, mut wire) = connect(&caps).await;
            let request = tokio::spawn(async move { request(&client, command).await });
            let tracked = labeled && batch;
            let prefix = if tracked { "@label=0 " } else { "" };
            if tracked {
                let expected = format!("{prefix}CHATHISTORY {command} ");
                let sent = wire.next_sent().await;
                assert!(sent.starts_with(&expected), "{sent}");
            }
            if batch {
                let (kind, row) = if command == "TARGETS" {
                    (
                        "draft/chathistory-targets",
                        ":offline.test CHATHISTORY TARGETS #papo 2025-10-15T11:55:42.123Z",
                    )
                } else {
                    (
                        "chathistory #papo",
                        ":alice!~a@cloak PRIVMSG #papo :earlier deployment",
                    )
                };
                wire.server(format!("{prefix}:offline.test BATCH +history {kind}"))
                    .await;
                wire.server(format!(
                    "@batch=history;time=2025-10-15T11:55:42.123Z;msgid=RKi9Cw {row}"
                ))
                .await;
                wire.server(":offline.test BATCH -history").await;
            }
            let result = request.await.unwrap();
            if tracked {
                assert_eq!(result.unwrap(), 1);
            } else {
                let missing = if labeled {
                    Feature::Batches
                } else {
                    Feature::LabeledResponse
                };
                assert!(
                    matches!(&result, Err(Error::Unsupported(error))
                        if error.feature == missing && error.reason == UnsupportedReason::NotOffered),
                    "{command}, {labeled}, {batch}: {result:?}"
                );
            }

            wire.server("PING :history-probe").await;
            assert_eq!(wire.next_sent().await, "PONG history-probe");
            wire.server(":alice!~a@cloak PRIVMSG papo :deployment complete")
                .await;
            let mut replays = 0;
            loop {
                match next_active_event(&mut events).await {
                    Event::History(page) => {
                        assert_eq!(page.items.len(), 1);
                        replays += 1;
                    }
                    Event::Message(_) => break,
                    _ => {}
                }
            }
            assert_eq!(
                replays,
                usize::from(batch && !labeled && command != "TARGETS")
            );
        }
    }
}
