use papo::{Endpoint, Sasl};
use papo_test_server::create_account;

use super::*;

#[tokio::test(flavor = "current_thread")]
async fn connects_with_sasl_and_applies_bot_defaults() {
    within(async {
        let server = from_env();
        let account = unique("papo-builder");
        let channel = format!("#{}", unique("papo-builder"));
        let password = "q7Hx-2mZp_9vLcT4wRfB";
        create_account(&server, &account, password).unwrap();

        let (client, mut events) = Client::builder(
            Endpoint::tls("localhost", server.tls_addr().port()).unwrap(),
            support::nick(&account),
        )
        .sasl(Sasl::plain(&account, papo::Secret::new(password)))
        .tls_only_ca_pem(std::fs::read(server.ca_pem_path()).unwrap())
        .bot_mode(true)
        .autojoin([&channel])
        .connect()
        .await
        .unwrap();

        wait_connected(&mut events, &account).await;
        wait_joined(&mut events, &account, &channel).await;

        assert_eq!(
            client
                .whois(support::nick(&account))
                .await
                .unwrap()
                .unwrap()
                .is_bot,
            Some(true)
        );
        client.quit().await.unwrap();
    })
    .await;
}

async fn wait_joined(events: &mut papo::Events, nick: &str, channel: &str) {
    loop {
        match events.next().await.map(|(event, _)| event) {
            Some(Event::Joined(papo::event::Joined {
                channel: joined,
                actor,
                ..
            })) if joined.name() == channel && actor.nick() == nick => return,
            Some(Event::Disconnected(disconnected)) => {
                panic!("disconnected while waiting for JOIN: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for JOIN"),
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn wrapped_connections_carry_the_tls_session() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    within(async {
        let server = from_env();
        let nick = unique("papo-wrapped");
        let opened = Arc::new(AtomicUsize::new(0));
        let (client, mut events) = papo::__testing::wrapped::connect_wrapped(
            Client::builder(
                Endpoint::tls("localhost", server.tls_addr().port()).unwrap(),
                support::nick(&nick),
            )
            .tls_only_ca_pem(std::fs::read(server.ca_pem_path()).unwrap()),
            {
                let opened = Arc::clone(&opened);
                move |stream| {
                    opened.fetch_add(1, Ordering::Relaxed);
                    stream
                }
            },
        )
        .await
        .unwrap();

        wait_connected(&mut events, &nick).await;
        assert_eq!(opened.load(Ordering::Relaxed), 1);
        client.quit().await.unwrap();
    })
    .await;
}
