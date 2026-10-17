//! Typed requests against the runner-managed Ergo server.
#![cfg(sockets)]

mod support;

use support::within;

use papo::{Client, Endpoint, Error, Event, ListFilter};
use papo_test_server::{from_env, unique};

#[tokio::test(flavor = "current_thread")]
async fn typed_requests_round_trip_against_ergo() {
    within(async {
        let server = from_env();
        let address = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        );
        let nick = unique("papo-requests");
        let channel = format!("#{}", unique("papo-requests"));
        let (client, mut events) = Client::builder(address)
            .nick(&nick)
            .connect()
            .await
            .unwrap();
        loop {
            match events.next().await {
                Some(Event::Registered(_)) => break,
                Some(Event::Disconnected(disconnected)) => {
                    panic!("disconnected: {disconnected}")
                }
                Some(_) => {}
                None => panic!("driver stopped during registration"),
            }
        }

        let whois = client.whois(&nick).await.unwrap();
        assert_eq!(whois.nick, nick.as_str());

        client.join(&channel).await.unwrap();
        assert_eq!(
            client.state().channel(&channel).unwrap().members().count(),
            1
        );
        assert!(
            client
                .who(&channel)
                .await
                .unwrap()
                .iter()
                .any(|entry| entry.nick == nick.as_str())
        );

        client.mode(&channel).add('t').await.unwrap();
        client
            .set_topic(&channel, "Typed requests on Ergo")
            .await
            .unwrap();
        assert_eq!(
            client.state().channel(&channel).unwrap().topic(),
            Some("Typed requests on Ergo")
        );
        assert!(
            client
                .list(ListFilter::default())
                .await
                .unwrap()
                .iter()
                .any(|entry| {
                    entry.name == channel.as_str() && entry.topic == "Typed requests on Ergo"
                })
        );

        let missing = unique("papo-missing");
        assert!(matches!(
            client.whois(&missing).await,
            Err(Error::Server(error)) if error.code() == "401"
        ));
        client
            .quit(Some("live request test complete"))
            .await
            .unwrap();
    })
    .await;
}
