//! End-to-end client behavior against the runner-managed Ergo server.
#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use std::time::Duration;

use papo::{Driver, Event, OsEntropy, transport};
use papo_core::{Config, Session};
use papo_test_server::{from_env, unique};
use tokio::time::timeout;

#[tokio::test(flavor = "current_thread")]
async fn registers_messages_itself_and_quits() {
    timeout(Duration::from_secs(10), async {
        let server = from_env();
        let nick = unique("papo-client");
        let stream = transport::connect(
            &server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
            &transport::Security::Plaintext,
        )
        .await
        .unwrap();
        let session = Session::with_entropy(
            Config::default()
                .nick(&nick)
                .username("papo")
                .realname("papo live client test")
                .host(server.plaintext_addr().ip().to_string()),
            Box::new(OsEntropy),
        );
        let (client, mut events, driver) = Driver::new(stream, session);
        let driver = tokio::spawn(driver);

        loop {
            match events.next().await.unwrap() {
                Event::Registered(welcome) => {
                    assert_eq!(welcome.nick, nick);
                    break;
                }
                Event::Disconnected { reason, .. } => {
                    panic!("disconnected while registering: {reason}")
                }
                _ => {}
            }
        }

        client
            .privmsg(&nick, "hello from papo's live client")
            .await
            .unwrap();
        loop {
            match events.next().await.unwrap() {
                Event::Message(message) => {
                    let message = message.as_ref();
                    if message.command_is("PRIVMSG")
                        && message.param(0) == Some(nick.as_str())
                        && message.param(1) == Some("hello from papo's live client")
                    {
                        break;
                    }
                }
                Event::Disconnected { reason, .. } => {
                    panic!("disconnected before echo: {reason}")
                }
                _ => {}
            }
        }

        client
            .quit(Some("live client test complete"))
            .await
            .unwrap();
        driver.await.unwrap().unwrap();
        loop {
            match events.next().await {
                Some(Event::Disconnected { .. }) => break,
                Some(_) => {}
                None => panic!("driver stopped without a disconnected event"),
            }
        }
    })
    .await
    .expect("live client did not finish within 10 seconds");
}
