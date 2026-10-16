//! Account APIs against the runner-managed Ergo server.
#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use std::time::Duration;

use papo::{AccountRegistration, Client, Event, Sasl};
use papo_core::Secret;
use papo_test_server::{from_env, unique};
use tokio::time::timeout;

const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

#[tokio::test(flavor = "current_thread")]
async fn registers_then_logs_in_with_sasl() {
    timeout(Duration::from_secs(10), async {
        let server = from_env();
        let address = format!("irc://{}", server.plaintext_addr());
        let account = unique("papo-account");
        let (registrar, mut registrar_events) = Client::builder(&address)
            .nick(&account)
            .caps(["draft/account-registration", "labeled-response"])
            .connect()
            .await
            .unwrap();
        wait_registered(&mut registrar_events, &account).await;
        assert!(matches!(
            registrar
                .register(Some(&account), None, Secret::new(PASSWORD))
                .await
                .unwrap(),
            AccountRegistration::Registered { account: registered } if registered == account
        ));
        registrar.quit(Some("account created")).await.unwrap();

        let (authenticated, mut authenticated_events) = Client::builder(&address)
            .nick(&account)
            .sasl(Sasl::plain(&account, PASSWORD))
            .require_sasl(true)
            .caps(["sasl"])
            .connect()
            .await
            .unwrap();
        wait_registered(&mut authenticated_events, &account).await;
        assert_eq!(
            authenticated.state(|state| state.account().map(str::to_owned)),
            Some(account.clone())
        );
        authenticated
            .quit(Some("SASL login verified"))
            .await
            .unwrap();
    })
    .await
    .expect("live account test did not finish within 10 seconds");
}

async fn wait_registered(events: &mut papo::Events, nick: &str) {
    loop {
        match events.next().await {
            Some(Event::Registered(welcome)) if welcome.nick == nick => return,
            Some(Event::Disconnected { reason, .. }) => panic!("disconnected: {reason}"),
            Some(_) => {}
            None => panic!("driver stopped before registration"),
        }
    }
}
