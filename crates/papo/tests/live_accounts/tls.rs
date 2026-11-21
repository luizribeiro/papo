use papo_test_server::create_account;

use super::*;

#[tokio::test(flavor = "current_thread")]
async fn tls_account_proof_produces_the_same_pin_on_refs_and_messages() {
    within(async {
        let server = from_env();
        let alice = unique("papo-peer-alice");
        let bob = unique("papo-peer-bob");
        let channel = format!("#{}", unique("papo-peer"));
        create_account(&server, &alice, PASSWORD).unwrap();
        create_account(&server, &bob, PASSWORD).unwrap();
        let endpoint = Endpoint::tls("localhost", server.tls_addr().port()).unwrap();
        let ca = std::fs::read(server.ca_pem_path()).unwrap();

        let (alice_client, mut alice_events) = Client::builder(endpoint.clone(), &alice)
            .sasl(Sasl::plain(&alice, papo::Secret::new(PASSWORD)))
            .tls_ca_pem(ca.clone())
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_connected(&mut alice_events, &alice).await;
        wait_joined(&mut alice_events, &alice, &channel).await;
        let (bob_client, mut bob_events) = Client::builder(endpoint, &bob)
            .sasl(Sasl::plain(&bob, papo::Secret::new(PASSWORD)))
            .tls_ca_pem(ca)
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_connected(&mut bob_events, &bob).await;
        wait_joined(&mut bob_events, &bob, &channel).await;
        wait_joined(&mut alice_events, &bob, &channel).await;

        alice_client
            .privmsg(&bob, "peer key live proof")
            .await
            .unwrap();
        let message =
            wait_account_message(&bob_client, &mut bob_events, &alice, "peer key live proof").await;
        let alice_ref = bob_client.query(&alice).unwrap();
        let message_key = message.actor.key();
        assert!(matches!(
            alice_ref.key().check(&message_key),
            papo::PeerKeyCheck::Match
        ));
        let stored = message_key.as_verified_str().unwrap();
        assert!(matches!(
            stored
                .parse::<papo::PeerKey>()
                .unwrap()
                .check(&alice_ref.key()),
            papo::PeerKeyCheck::Match
        ));

        alice_client.quit().await.unwrap();
        bob_client.quit().await.unwrap();
    })
    .await;
}

async fn wait_account_message(
    client: &Client,
    events: &mut papo::Events,
    sender: &str,
    text: &str,
) -> papo::Message {
    let mut received = None;
    loop {
        match events.next().await.map(|(event, _)| event) {
            Some(Event::Message(message)) if message.text == text => received = Some(message),
            Some(Event::Disconnected(disconnected)) => panic!("disconnected: {disconnected}"),
            Some(_) => {}
            None => panic!("driver stopped before account proof and private message"),
        }
        let account_proved = client.snapshot().user(sender).unwrap().is_some_and(
            |user| matches!(user.account(), papo::Account::LoggedIn(account) if account == sender),
        );
        if account_proved && let Some(message) = received.take() {
            return message;
        }
    }
}

async fn wait_joined(events: &mut papo::Events, nick: &str, channel: &str) {
    loop {
        match events.next().await.map(|(event, _)| event) {
            Some(Event::Joined(joined))
                if joined.actor.nick() == nick && joined.channel == channel =>
            {
                return;
            }
            Some(Event::Disconnected(disconnected)) => panic!("disconnected: {disconnected}"),
            Some(_) => {}
            None => panic!("driver stopped before channel join"),
        }
    }
}
