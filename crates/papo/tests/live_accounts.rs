//! Account APIs against the runner-managed Ergo server.
#![cfg(sockets)]
#![cfg(feature = "unstable")]

mod support;

use support::within;

use papo::{AccountCreation, Client, Endpoint, Event, NewAccount, Sasl};
use papo_test_server::{create_account, from_env, unique};

const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

#[tokio::test(flavor = "current_thread")]
async fn registers_then_logs_in_with_sasl() {
    within(async {
        let server = from_env();
        let address = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        )
        .unwrap();
        let account = unique("papo-account");
        let (registrar, mut registrar_events) =
            Client::builder(address.clone(), support::nick(&account))
                .allow_insecure_auth(true)
                .connect()
                .await
                .unwrap();
        wait_connected(&mut registrar_events, &account).await;
        assert!(matches!(
            registrar
                .create_account(
                    NewAccount::new(papo::Secret::new(PASSWORD))
                        .account(support::account(&account)),
                )
                .await
                .unwrap(),
            AccountCreation::Created {
                account: registered,
                ..
            } if registered == account
        ));
        registrar.quit("account created").await.unwrap();

        let (authenticated, mut authenticated_events) =
            Client::builder(address, support::nick(&account))
                .allow_insecure_auth(true)
                .sasl(Sasl::plain(&account, papo::Secret::new(PASSWORD)))
                .connect()
                .await
                .unwrap();
        wait_connected(&mut authenticated_events, &account).await;
        assert!(matches!(
            authenticated.snapshot().account(),
            papo::Account::LoggedIn(logged_in)
                if logged_in == account.as_str()
                    && logged_in.origin() == authenticated.origin()
        ));
        authenticated.quit("SASL login verified").await.unwrap();
    })
    .await;
}

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

        let bob_ref = alice_client.direct(&bob).await.unwrap();
        alice_client
            .privmsg(&bob_ref, "peer key live proof")
            .await
            .unwrap();
        let message =
            wait_account_message(&bob_client, &mut bob_events, &alice, "peer key live proof").await;
        let alice_ref = bob_client.direct(&alice).await.unwrap();
        assert!(matches!(
            alice_ref
                .peer_key()
                .check(message.actor.peer_key().unwrap()),
            papo::PeerKeyCheck::Match
        ));
        let stored = message.actor.peer_key().unwrap().as_verified_str().unwrap();
        assert!(matches!(
            stored
                .parse::<papo::PeerKey>()
                .unwrap()
                .check(alice_ref.peer_key()),
            papo::PeerKeyCheck::Match
        ));

        alice_client.quit("peer key verified").await.unwrap();
        bob_client.quit("peer key verified").await.unwrap();
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
        match events.next().await {
            Some(Event::Message(message)) if message.text == text => received = Some(message),
            Some(Event::Disconnected(disconnected)) => panic!("disconnected: {disconnected}"),
            Some(_) => {}
            None => panic!("driver stopped before account proof and private message"),
        }
        let account_proved = client.snapshot().user(sender).is_some_and(
            |user| matches!(user.account(), papo::Account::LoggedIn(account) if account == sender),
        );
        if account_proved && let Some(message) = received.take() {
            return message;
        }
    }
}

async fn wait_connected(events: &mut papo::Events, nick: &str) {
    loop {
        match events.next().await {
            Some(Event::Connected(welcome)) if welcome.nick == nick => return,
            Some(Event::Disconnected(disconnected)) => {
                panic!("disconnected: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped before registration"),
        }
    }
}

async fn wait_joined(events: &mut papo::Events, nick: &str, channel: &str) {
    loop {
        match events.next().await {
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
