//! Account APIs against the runner-managed Ergo server.
#![cfg(sockets)]
#![cfg(feature = "unstable")]

mod support;

use support::within;

use papo::{AccountOutcome, Client, Endpoint, Event, NewAccount, Sasl};
use papo_test_server::{create_account, from_env, unique};

#[cfg(tls)]
#[path = "live_accounts/tls.rs"]
mod tls;

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
            AccountOutcome::Created {
                account: registered,
                ..
            } if registered == account
        ));
        registrar.quit().await.unwrap();

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
        authenticated.quit().await.unwrap();
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

async fn wait_connected(events: &mut papo::Events, nick: &str) {
    loop {
        match events.next().await.map(|(event, _)| event) {
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
