//! Typed requests against the runner-managed Ergo server.
#![cfg(sockets)]

mod support;

use support::within;

use papo::{Client, Endpoint, Event};
use papo_test_server::{from_env, unique};

#[tokio::test(flavor = "current_thread")]
async fn typed_requests_round_trip_against_ergo() {
    within(async {
        let server = from_env();
        let address = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        )
        .unwrap();
        let nick = unique("papo-requests");
        let channel = format!("#{}", unique("papo-requests"));
        let (client, mut events) = Client::builder(address, support::nick(&nick))
            .connect()
            .await
            .unwrap();
        loop {
            match events.next().await.map(|(event, _)| event) {
                Some(Event::Connected(_)) => break,
                Some(Event::Disconnected(disconnected)) => {
                    panic!("disconnected: {disconnected}")
                }
                Some(_) => {}
                None => panic!("driver stopped during registration"),
            }
        }

        client
            .check_connection(std::time::Duration::from_secs(3))
            .await
            .unwrap();
        assert!(matches!(
            client.reconnect().await,
            Err(papo::Error::AlreadyConnected)
        ));

        let whois = client.whois(support::nick(&nick)).await.unwrap().unwrap();
        assert_eq!(whois.user.nick(), nick.as_str());

        client.join(support::channel(&channel)).await.unwrap();
        assert_eq!(
            client
                .snapshot()
                .channel(&channel)
                .unwrap()
                .unwrap()
                .members()
                .count(),
            1
        );
        assert!(
            client
                .who(support::mask(&channel).to_string())
                .await
                .unwrap()
                .iter()
                .any(|entry| entry.user.nick() == nick.as_str())
        );

        client.set_modes(&channel).add('t').await.unwrap();
        client
            .set_topic(support::channel(&channel), "Typed requests on Ergo")
            .await
            .unwrap();
        assert_eq!(
            client
                .snapshot()
                .channel(&channel)
                .unwrap()
                .unwrap()
                .topic(),
            Some("Typed requests on Ergo")
        );
        assert!(client.list().await.unwrap().iter().any(|entry| {
            entry.name == channel.as_str()
                && entry.topic.as_deref() == Some("Typed requests on Ergo")
        }));

        let missing = unique("papo-missing");
        assert_eq!(client.whois(support::nick(&missing)).await.unwrap(), None);
        client.quit().await.unwrap();
    })
    .await;
}
