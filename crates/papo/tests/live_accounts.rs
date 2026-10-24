//! Account APIs against the runner-managed Ergo server.
#![cfg(sockets)]
#![cfg(feature = "unstable")]

mod support;

use support::within;

use papo::{AccountCreation, Client, Endpoint, Event, NewAccount, Sasl};
use papo_test_server::{from_env, unique};

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
        let (registrar, mut registrar_events) = Client::builder(address.clone())
            .nick(support::nick(&account))
            .allow_insecure_auth(true)
            .request_caps(["draft/account-registration", "labeled-response"])
            .connect()
            .await
            .unwrap();
        wait_connected(&mut registrar_events, &account).await;
        assert!(matches!(
            registrar
                .create_account(NewAccount::new(PASSWORD).account(support::account(&account)))
                .await
                .unwrap(),
            AccountCreation::Created {
                account: registered,
                ..
            } if registered.as_ref() == account
        ));
        registrar.quit("account created").await.unwrap();

        let (authenticated, mut authenticated_events) = Client::builder(address)
            .nick(support::nick(&account))
            .allow_insecure_auth(true)
            .sasl(Sasl::plain(&account, PASSWORD))
            .request_caps(["sasl"])
            .connect()
            .await
            .unwrap();
        wait_connected(&mut authenticated_events, &account).await;
        assert!(matches!(
            authenticated.state().account(),
            papo::Account::LoggedIn(logged_in)
                if logged_in == account.as_str()
                    && logged_in.origin() == authenticated.origin()
        ));
        authenticated.quit("SASL login verified").await.unwrap();
    })
    .await;
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
