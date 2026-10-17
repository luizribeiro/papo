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
                Some(papo::__papo_disconnected!(reason, _)) => panic!("disconnected: {reason}"),
                Some(_) => {}
                None => panic!("driver stopped during registration"),
            }
        }

        let whois = client.whois(&nick).await.unwrap();
        assert_eq!(whois.nick, nick);

        let joined = client.join(&channel).await.unwrap();
        assert_eq!(joined.name, channel);
        assert_eq!(joined.members_count, 1);
        assert!(
            client
                .who(&channel)
                .await
                .unwrap()
                .iter()
                .any(|entry| entry.nick == nick)
        );

        client.mode(&channel, "+t", [] as [&str; 0]).await.unwrap();
        assert_eq!(
            client
                .topic(&channel, Some("Typed requests on Ergo"))
                .await
                .unwrap()
                .as_deref(),
            Some("Typed requests on Ergo")
        );
        assert_eq!(
            client.topic(&channel, None).await.unwrap().as_deref(),
            Some("Typed requests on Ergo")
        );
        assert!(
            client
                .list(ListFilter::default())
                .await
                .unwrap()
                .iter()
                .any(|entry| { entry.name == channel && entry.topic == "Typed requests on Ergo" })
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
